use crate::{
    file,
    fleet::{MANIFEST, Manifest},
    prompt::Prompt,
    secrets, sign, user,
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
        bail!("the fleet has a recovery key already");
    }
    let answers = [('n', From::New), ('t', From::Typed)];
    let key = match prompt.ask("n make a new one · t type the one you have ›", &answers)? {
        None => bail!("the fleet still has no recovery key"),
        Some(From::New) => {
            let key = x25519::Identity::generate();
            println!(
                "The recovery key opens every secret, signs any change, and nothing else keeps \
                 it. Write it on paper and in your password manager:"
            );
            println!("  {}", key.to_string().expose_secret());
            key
        }
        Some(From::Typed) => secrets::identity(&prompt.hidden("Recovery key:")?)?,
    };
    let path = dir.join(MANIFEST);
    let mut doc: DocumentMut = file::read_if_any(&path)?
        .unwrap_or_default()
        .parse()
        .with_context(|| format!("parsing {}", path.display()))?;
    let mut recovery = Table::new();
    recovery.insert("age", secrets::public(&key).into());
    recovery.insert("signing", sign::public(&sign::recovery_key(&key))?.into());
    doc.insert("recovery", Item::Table(recovery));
    file::write(&path, doc.to_string().as_bytes(), 0o644)?;
    user::by_uid(fs::metadata(dir)?.uid())?.own(&path)?;
    println!("The fleet's {MANIFEST} names it now. Commit and push the fleet.");
    Ok(())
}

#[cfg(test)]
#[path = "../tests/recovery.rs"]
mod tests;
