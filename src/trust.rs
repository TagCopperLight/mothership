use crate::{
    fleet::{self, Fleet},
    homes,
    secrets::{
        AGE, Keys, RECIPIENTS, RECOVERY, ROOT_KEY, Recipient, SECRETS, Storage, encrypt_for,
        format, holds, key_path, make_key, public, read, root_key, user_key,
    },
    walk,
};
use age::x25519;
use anyhow::{Context, Result, anyhow, bail};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

const ENCRYPTED: &str = "encrypted_";

pub struct MachineKey {
    pub name: String,
    pub public: String,
    pub target: String,
    pub list: PathBuf,
    pub trusted: bool,
}

pub fn machine_keys(fleet: &Fleet) -> Result<Vec<MachineKey>> {
    let mut owners = vec![("root".to_owned(), None, fleet.recipients())];
    for member in &fleet.members {
        if let Some(user) = &member.user {
            owners.push((member.name.clone(), Some(user), member.recipients()));
        }
    }
    owners
        .into_iter()
        .map(|(name, user, list)| {
            let key = match user {
                None => match root_key()? {
                    Some(key) => key,
                    None => make_key(Path::new(ROOT_KEY))?,
                },
                Some(user) => match user_key(user)? {
                    Some(key) => key,
                    None => user.with_rights(|| make_key(&key_path(user)))?,
                },
            };
            let public = public(&key);
            Ok(MachineKey {
                target: format!("{}/{name}", fleet.type_name),
                trusted: holds(&list, &public)?,
                name,
                public,
                list,
            })
        })
        .collect()
}

pub fn key(fleet: &Fleet) -> Result<()> {
    let keys = machine_keys(fleet)?;
    for key in &keys {
        println!("{:6}  {}", key.name, key.public);
    }
    let untrusted: Vec<&MachineKey> = keys.iter().filter(|k| !k.trusted).collect();
    if !untrusted.is_empty() {
        println!("Add them from a machine that opens the secrets, or here for the first one:");
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
    User(String),
}

struct Target {
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
            let dir = type_dir.join(fleet::USERS).join(user);
            if user.is_empty() || user.contains('/') || user.contains("..") || !dir.is_dir() {
                bail!("{type_name} has no user {user}");
            }
            (Scope::User(user.to_owned()), dir.join(RECIPIENTS))
        }
    };
    Ok(Target {
        type_name: type_name.to_owned(),
        scope,
        list,
    })
}

pub fn trust(fleet: &Fleet, text: &str, name: &str, key: &str) -> Result<()> {
    trust_with(fleet, fleet.keys(), text, name, key)
}

pub fn trust_with(fleet: &Fleet, keys: &Keys, text: &str, name: &str, key: &str) -> Result<()> {
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
    if name == RECOVERY && list.iter().any(|r| r.name == RECOVERY) {
        bail!("{text} already has a recovery key: retire it first");
    }
    let before = list.len();
    list.retain(|r| r.name != name);
    if list.len() < before {
        println!("This key replaces {name}'s old one.");
    }
    list.push(Recipient {
        name: name.to_owned(),
        key: key.to_owned(),
    });
    let rewritten = rewrite(fleet, keys, &target, &list)?;
    println!(
        "{name} opens the {} secrets of {text} now. Commit and push the fleet.",
        rewritten.len()
    );
    Ok(())
}

pub fn retire(fleet: &Fleet, text: &str, name: &str) -> Result<()> {
    let target = target(fleet, text)?;
    let mut list = read(&target.list)?;
    let retired = list
        .iter()
        .find(|r| r.name == name)
        .with_context(|| format!("{text} holds no {name}"))?
        .key
        .clone();
    list.retain(|r| r.name != name);
    if list.is_empty() {
        bail!("{name} is the last key of {text}: trust another one first");
    }
    let rewritten = rewrite(fleet, fleet.keys(), &target, &list)?;
    println!("{name} is gone from {text}. Commit and push the fleet.");
    if rewritten.is_empty() {
        return Ok(());
    }
    println!(
        "git history keeps the versions {name} opens, and that machine may keep copies, so change these:"
    );
    for (shown, _) in &rewritten {
        println!("  {shown}");
    }
    let still: Vec<&str> = rewritten
        .iter()
        .filter(|(_, recipients)| recipients.iter().any(|r| r.key == retired))
        .map(|(shown, _)| shown.as_str())
        .collect();
    if !still.is_empty() {
        println!("{name}'s key still opens these, through another type's list for the same user:");
        for shown in still {
            println!("  {shown}");
        }
    }
    Ok(())
}

pub fn home_list(fleet_dir: &Path, source: &Path) -> Result<String> {
    Ok(format(&home_recipients(fleet_dir, source, None)?))
}

fn home_recipients(
    fleet_dir: &Path,
    source: &Path,
    replacing: Option<(&Path, &[Recipient])>,
) -> Result<Vec<Recipient>> {
    let mut all: Vec<Recipient> = Vec::new();
    for user in homes::users(fleet_dir, source)? {
        for type_name in fleet::types(fleet_dir)? {
            let dir = fleet::type_dir(fleet_dir, &type_name)
                .join(fleet::USERS)
                .join(&user);
            if !dir.is_dir() {
                continue;
            }
            let path = dir.join(RECIPIENTS);
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
    }
    Ok(all)
}

fn rewrite(
    fleet: &Fleet,
    keys: &Keys,
    target: &Target,
    list: &[Recipient],
) -> Result<Vec<(String, Vec<Recipient>)>> {
    let all = secrets_of(fleet, target)?;
    let mut sealed = Vec::new();
    for (shown, source) in &all {
        let recipients = match source {
            Source::Home(path) => home_recipients(&fleet.dir, path, Some((&target.list, list)))?,
            Source::List(_) => list.to_vec(),
        };
        let file = source.file(&fleet.dir)?;
        let bytes = encrypt_for(&recipients, &keys.decrypt(&file)?)?;
        sealed.push((shown.clone(), file, bytes, recipients));
    }
    let mut rewritten = Vec::new();
    for (shown, file, bytes, recipients) in sealed {
        let mode = fs::metadata(&file)?.permissions().mode() & 0o7777;
        walk::write(&file, &bytes, mode).with_context(|| format!("writing {}", file.display()))?;
        rewritten.push((shown, recipients));
    }
    walk::write(&target.list, format(list).as_bytes(), 0o644)
        .with_context(|| format!("writing {}", target.list.display()))?;
    Ok(rewritten)
}

enum Source {
    Home(PathBuf),
    List(PathBuf),
}

impl Source {
    fn file(&self, fleet_dir: &Path) -> Result<PathBuf> {
        match self {
            Source::Home(path) if fs::symlink_metadata(path)?.is_symlink() => {
                homes::target(fleet_dir, path)
            }
            Source::Home(path) | Source::List(path) => Ok(path.clone()),
        }
    }
}

fn secrets_of(fleet: &Fleet, target: &Target) -> Result<BTreeMap<String, Source>> {
    let type_dir = fleet::type_dir(&fleet.dir, &target.type_name);
    let mut all = BTreeMap::new();
    let files = |dir: &Path| match dir.exists() {
        true => walk::files(dir),
        false => Ok(Vec::new()),
    };
    match &target.scope {
        Scope::Root => {
            for file in fleet::etc_files(&type_dir)? {
                if file.storage == Storage::Encrypted {
                    let path = Path::new(walk::ETC).join(&file.rel);
                    all.insert(path.display().to_string(), Source::List(file.source));
                }
            }
        }
        Scope::User(user) => {
            let home = homes::source(&fleet.dir, user);
            for rel in files(&home)? {
                if let Some(shown) = home_secret(&rel) {
                    all.insert(shown, Source::Home(home.join(&rel)));
                }
            }
            let dir = type_dir.join(fleet::USERS).join(user).join(SECRETS);
            for rel in files(&dir)? {
                let source = dir.join(&rel);
                let shown = source.strip_prefix(&fleet.dir)?.display().to_string();
                all.insert(shown, Source::List(source));
            }
        }
    }
    Ok(all)
}

const ATTRIBUTES: [&str; 11] = [
    "create_",
    "modify_",
    "remove_",
    ENCRYPTED,
    "private_",
    "readonly_",
    "empty_",
    "executable_",
    "exact_",
    "external_",
    "symlink_",
];

fn home_secret(rel: &Path) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut encrypted = false;
    for part in rel.iter() {
        let mut name = part.to_str()?;
        while let Some(rest) = ATTRIBUTES.iter().find_map(|a| name.strip_prefix(a)) {
            encrypted |= name.starts_with(ENCRYPTED);
            name = rest;
        }
        let name = match name.strip_prefix("dot_") {
            Some(rest) => format!(".{rest}"),
            None => name.to_owned(),
        };
        parts.push(name);
    }
    let last = parts.last_mut()?;
    while let Some(rest) = last
        .strip_suffix(AGE)
        .or_else(|| last.strip_suffix(".tmpl"))
    {
        *last = rest.to_owned();
    }
    encrypted.then(|| format!("~/{}", parts.join("/")))
}

#[cfg(test)]
#[path = "../tests/trust.rs"]
mod tests;
