use crate::{
    file,
    fleet::{
        self, Fleet, MANIFEST, Manifest,
        checked::{self, Advance, Outcome, Remote},
    },
    prompt::Prompt,
    secrets::{self, Keys},
    sign, trust, user,
};
use age::{secrecy::ExposeSecret, x25519};
use anyhow::{Context, Result, bail};
use std::{fs, os::unix::fs::MetadataExt, path::Path};
use toml_edit::{DocumentMut, Item, Table};

#[derive(Clone, Copy)]
enum From {
    New,
    Typed,
}

pub fn run(dir: &Path, prompt: &mut Prompt) -> Result<()> {
    if Manifest::read(dir)?.recovery().is_some() {
        return change(prompt);
    }
    let key = new_key(prompt)?;
    let path = dir.join(MANIFEST);
    name(&path, &key)?;
    user::by_uid(fs::metadata(dir)?.uid())?.own(&path)?;
    println!("The fleet's {MANIFEST} names it now. Commit and push the fleet.");
    Ok(())
}

fn new_key(prompt: &mut Prompt) -> Result<x25519::Identity> {
    let answers = [('n', From::New), ('t', From::Typed)];
    match prompt.ask("n make a new one · t type the one you have ›", &answers)? {
        None => bail!("the fleet's recovery key stays as it was"),
        Some(From::New) => {
            let key = x25519::Identity::generate();
            println!(
                "The recovery key opens every secret, signs any change, and nothing else keeps \
                 it. Write it on paper and in your password manager:"
            );
            println!("  {}", key.to_string().expose_secret());
            Ok(key)
        }
        Some(From::Typed) => secrets::identity(&prompt.hidden("Recovery key:")?),
    }
}

fn name(path: &Path, key: &x25519::Identity) -> Result<()> {
    let mut doc: DocumentMut = file::read_if_any(path)?
        .unwrap_or_default()
        .parse()
        .with_context(|| format!("parsing {}", path.display()))?;
    let mut recovery = Table::new();
    recovery.insert("age", secrets::public(key).into());
    recovery.insert("signing", sign::public(&sign::recovery_key(key))?.into());
    doc.insert("recovery", Item::Table(recovery));
    file::write(path, doc.to_string().as_bytes(), 0o644)
}

fn change(prompt: &mut Prompt) -> Result<()> {
    let admin = fleet::admin()?;
    checked::sync(&admin, Remote::Fetch)?;
    let stage = checked::stage()?;
    let old = secrets::identity(&prompt.hidden("The fleet's recovery key now:")?)?;
    let old_public = secrets::public(&old);
    if old_public != Fleet::load_at(&stage.dir)?.recovery.key {
        bail!("that isn't the recovery key the fleet's {MANIFEST} names");
    }
    println!("The new recovery key:");
    let new = new_key(prompt)?;
    if secrets::public(&new) == old_public {
        bail!("that's the recovery key the fleet has already");
    }
    name(&stage.dir.join(MANIFEST), &new)?;
    let fleet = Fleet::load_at(&stage.dir)?;
    let mut keys = Keys::default();
    keys.push(old.clone());
    trust::reseal_everything(&fleet, &keys, &old_public)?;
    let key = sign::recovery_key(&old);
    let message = "Change the recovery key";
    if let Outcome::Pushed(_) = stage.commit(&admin, message, &key, prompt, Advance::Merge)? {
        println!("The old recovery key opens what git history keeps, so change those secrets too.");
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/recovery.rs"]
mod tests;
