use crate::{
    chezmoi, file,
    fleet::{
        self, Fleet, Repo,
        homes::{self, Home},
    },
    prompt::Prompt,
    secrets::{
        AGE, Keys, RECIPIENTS, ROOT_KEY, Recipient, Storage, encrypt_for, format, holds, key_path,
        make_key, public, read, root_key, user_key,
    },
    style,
    user::User,
    walk,
};
use age::x25519;
use anyhow::{Context, Result, anyhow, bail};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub struct MachineKey {
    pub name: String,
    pub public: String,
    pub target: String,
    pub list: PathBuf,
    pub trusted: bool,
}

fn owners(fleet: &Fleet) -> Vec<(String, Option<&User>, PathBuf)> {
    let mut owners = vec![("root".to_owned(), None, fleet.recipients())];
    for member in &fleet.members {
        owners.push((
            member.name.clone(),
            Some(member.user()),
            member.recipients(),
        ));
    }
    owners
}

pub fn create_keys(fleet: &Fleet) -> Result<()> {
    for (_, user, _) in owners(fleet) {
        match user {
            None if root_key()?.is_none() => drop(make_key(Path::new(ROOT_KEY))?),
            Some(user) if user_key(user)?.is_none() => {
                user.with_rights(|| make_key(&key_path(user)))?;
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn machine_keys(fleet: &Fleet) -> Result<Vec<MachineKey>> {
    let mut keys = Vec::new();
    for (name, user, list) in owners(fleet) {
        let key = match user {
            None => root_key()?,
            Some(user) => user_key(user)?,
        };
        let Some(key) = key else {
            continue;
        };
        let public = public(&key);
        keys.push(MachineKey {
            target: format!("{}/{name}", fleet.type_name),
            trusted: holds(&list, &public)?,
            name,
            public,
            list,
        });
    }
    Ok(keys)
}

pub fn key(fleet: &Fleet) -> Result<()> {
    create_keys(fleet)?;
    let keys = machine_keys(fleet)?;
    for key in &keys {
        println!("{:6}  {}", key.name, key.public);
    }
    let untrusted: Vec<&MachineKey> = keys.iter().filter(|k| !k.trusted).collect();
    if !untrusted.is_empty() {
        println!(
            "Add them from a machine that opens their secrets, or here for a list that guards none:"
        );
        for key in untrusted {
            println!("  {}", trust_command(&key.target, "<name>", &key.public));
        }
    }
    Ok(())
}

pub fn trust_command(target: &str, name: &str, public: &str) -> String {
    format!("sudo mothership trust {target} {name} {public}")
}

enum Scope {
    Root,
    User(Home),
}

struct Target {
    text: String,
    type_name: String,
    scope: Scope,
    list: PathBuf,
}

fn target(fleet: &Fleet, text: &str) -> Result<Target> {
    let (type_name, scope) = text
        .split_once('/')
        .with_context(|| format!("{text} is not <type>/root or <type>/<user>"))?;
    let type_dir = fleet::type_dir(&fleet.dir, type_name);
    if type_name.is_empty() || type_name.contains("..") || !type_dir.is_dir() {
        bail!("the fleet has no type {type_name}");
    }
    let (scope, list) = match scope {
        "root" => (Scope::Root, type_dir.join(RECIPIENTS)),
        user => {
            let home = Home {
                type_name: type_name.to_owned(),
                user: user.to_owned(),
            };
            let dir = home.dir(&fleet.dir);
            if user.is_empty() || user.contains('/') || user.contains("..") || !dir.is_dir() {
                bail!("{type_name} has no user {user}");
            }
            (Scope::User(home), dir.join(RECIPIENTS))
        }
    };
    Ok(Target {
        text: text.to_owned(),
        type_name: type_name.to_owned(),
        scope,
        list,
    })
}

pub fn guards_secrets(fleet: &Fleet, text: &str) -> Result<bool> {
    Ok(!secrets_of(fleet, &target(fleet, text)?)?.is_empty())
}

pub fn trust(fleet: &Fleet, text: &str, name: &str, key: &str, prompt: &mut Prompt) -> Result<()> {
    trust_with(fleet, fleet.keys(), text, name, key, prompt)
}

pub fn trust_with(
    fleet: &Fleet,
    keys: &Keys,
    text: &str,
    name: &str,
    key: &str,
    prompt: &mut Prompt,
) -> Result<()> {
    let target = target(fleet, text)?;
    key.parse::<x25519::Recipient>()
        .map_err(|e| anyhow!("{key}: {e}"))?;
    if name.is_empty() || name.contains(['\n', '#']) {
        bail!("a key's name can't be empty or hold a newline or #");
    }
    let mut list = read(&target.list)?;
    if let Some(known) = list.iter().find(|r| r.key == key) {
        bail!("{text} already holds this key, as {}", known.name);
    }
    if list.iter().any(|r| r.name == name) {
        bail!(
            "{text} already holds a key named {name}: run `mothership retire {text} {name}` first"
        );
    }
    list.push(Recipient {
        name: name.to_owned(),
        key: key.to_owned(),
    });
    let Some(rewritten) = rewrite(fleet, keys, &target, &list, prompt)? else {
        println!("Nothing changed.");
        return Ok(());
    };
    println!(
        "{name} opens the {} secrets of {text} now. Commit and push the fleet.",
        rewritten.len()
    );
    Ok(())
}

pub fn retire(fleet: &Fleet, text: &str, name: &str, prompt: &mut Prompt) -> Result<()> {
    let target = target(fleet, text)?;
    let mut list = read(&target.list)?;
    let retired = list
        .iter()
        .find(|r| r.name == name)
        .with_context(|| format!("{text} holds no {name}"))?
        .key
        .clone();
    list.retain(|r| r.name != name);
    let Some(rewritten) = rewrite(fleet, fleet.keys(), &target, &list, prompt)? else {
        println!("Nothing changed.");
        return Ok(());
    };
    println!("{name} is gone from {text}. Commit and push the fleet.");
    if rewritten.is_empty() {
        return Ok(());
    }
    println!(
        "git history keeps the versions {name} opens, and that machine may keep copies, so change these:"
    );
    for secret in &rewritten {
        println!("  {}", secret.shown);
    }
    let still: Vec<&Rewritten> = rewritten
        .iter()
        .filter(|secret| secret.recipients.iter().any(|r| r.key == retired))
        .collect();
    if !still.is_empty() {
        println!("{name}'s key still opens these, through another home that shares them:");
        let others = |secret: &Rewritten| {
            let others: Vec<&str> = secret
                .readers
                .iter()
                .map(String::as_str)
                .filter(|reader| *reader != text)
                .collect();
            others.join(", ")
        };
        let pairs: Vec<(String, String)> = still
            .iter()
            .map(|secret| (secret.shown.clone(), others(secret)))
            .collect();
        style::print_pairs(&pairs);
    }
    Ok(())
}

pub fn home_list(repo: &Repo, source: &Path) -> Result<String> {
    Ok(format(
        &repo.sealed_for(home_recipients(&repo.dir, source, None)?),
    ))
}

fn home_recipients(
    fleet_dir: &Path,
    source: &Path,
    replacing: Option<(&Path, &[Recipient])>,
) -> Result<Vec<Recipient>> {
    let mut all: Vec<Recipient> = Vec::new();
    for home in homes::owners(fleet_dir, source)? {
        let path = home.dir(fleet_dir).join(RECIPIENTS);
        let list = match replacing {
            Some((replaced, list)) if replaced == path => list.to_vec(),
            _ => read(&path)?,
        };
        for recipient in list {
            if !all.iter().any(|known| known.key == recipient.key) {
                all.push(recipient);
            }
        }
    }
    Ok(all)
}

struct Rewritten {
    shown: String,
    readers: Vec<String>,
    recipients: Vec<Recipient>,
}

fn rewrite(
    fleet: &Fleet,
    keys: &Keys,
    target: &Target,
    list: &[Recipient],
    prompt: &mut Prompt,
) -> Result<Option<Vec<Rewritten>>> {
    let all = secrets_of(fleet, target)?;
    if !all.is_empty() {
        println!("Re-encrypting for {}:", target.text);
        let pairs: Vec<(String, String)> = all
            .iter()
            .map(|secret| (secret.shown.clone(), secret.readers.join(", ")))
            .collect();
        style::print_pairs(&pairs);
        if !prompt.confirm("Go on")? {
            return Ok(None);
        }
    }
    let mut sealed = Vec::new();
    for secret in all {
        let recipients = fleet.sealed_for(match &secret.source {
            Source::Home(path) => home_recipients(&fleet.dir, path, Some((&target.list, list)))?,
            Source::Etc(_) => list.to_vec(),
        });
        let file = secret.source.file(&fleet.dir)?;
        let bytes = encrypt_for(&recipients, &keys.decrypt(&file)?)?;
        sealed.push((file, bytes, secret, recipients));
    }
    let mut rewritten = Vec::new();
    for (file, bytes, secret, recipients) in sealed {
        let mode = fs::metadata(&file)?.permissions().mode() & 0o7777;
        file::write(&file, &bytes, mode).with_context(|| format!("writing {}", file.display()))?;
        rewritten.push(Rewritten {
            shown: secret.shown,
            readers: secret.readers,
            recipients,
        });
    }
    file::write(&target.list, format(list).as_bytes(), 0o644)
        .with_context(|| format!("writing {}", target.list.display()))?;
    Ok(Some(rewritten))
}

enum Source {
    Home(PathBuf),
    Etc(PathBuf),
}

impl Source {
    fn file(&self, fleet_dir: &Path) -> Result<PathBuf> {
        match self {
            Source::Home(path) if fs::symlink_metadata(path)?.is_symlink() => {
                homes::target(fleet_dir, path)
            }
            Source::Home(path) | Source::Etc(path) => Ok(path.clone()),
        }
    }
}

struct Secret {
    shown: String,
    source: Source,
    readers: Vec<String>,
}

fn secrets_of(fleet: &Fleet, target: &Target) -> Result<Vec<Secret>> {
    let mut all = Vec::new();
    match &target.scope {
        Scope::Root => {
            let type_dir = fleet::type_dir(&fleet.dir, &target.type_name);
            for file in fleet::etc_files(&type_dir)? {
                if file.storage == Storage::Encrypted {
                    all.push(Secret {
                        shown: Path::new(file::ETC).join(&file.rel).display().to_string(),
                        source: Source::Etc(file.source),
                        readers: vec![target.text.clone()],
                    });
                }
            }
        }
        Scope::User(home) => {
            let source = home.source(&fleet.dir);
            for rel in walk::files(&source)? {
                if let Some(shown) = home_secret(&rel) {
                    let path = source.join(&rel);
                    let readers = homes::owners(&fleet.dir, &path)?
                        .iter()
                        .map(Home::to_string)
                        .collect();
                    all.push(Secret {
                        shown,
                        source: Source::Home(path),
                        readers,
                    });
                }
            }
        }
    }
    all.sort_by(|a, b| a.shown.cmp(&b.shown));
    Ok(all)
}

fn home_secret(rel: &Path) -> Option<String> {
    let secret = match rel.starts_with(homes::SECRETS) {
        true => rel.to_string_lossy().ends_with(AGE),
        false => chezmoi::target_of(rel).1,
    };
    secret.then(|| homes::shown(rel))
}

#[cfg(test)]
#[path = "../tests/trust.rs"]
mod tests;
