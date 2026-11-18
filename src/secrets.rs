use crate::{chezmoi, fleet::Fleet, user::User, walk};
use age::{
    armor::{ArmoredReader, ArmoredWriter, Format},
    secrecy::ExposeSecret,
    x25519,
};
use anyhow::{Context, Result, anyhow, bail};
use std::{
    collections::BTreeMap,
    fs,
    io::{ErrorKind, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

pub const RECIPIENTS: &str = "recipients.txt";
pub const ROOT_KEY: &str = "/etc/mothership/key.txt";
pub const SECRETS: &str = "secrets";
pub const AGE: &str = ".age";

pub fn key_path(user: &User) -> PathBuf {
    user.home.join(".config/mothership/key.txt")
}

pub struct Keys(Vec<x25519::Identity>);

impl Keys {
    pub fn load(paths: impl Iterator<Item = PathBuf>) -> Result<Keys> {
        let mut keys = Vec::new();
        for path in paths {
            keys.extend(read_key(&path)?);
        }
        Ok(Keys(keys))
    }

    pub fn decrypt(&self, path: &Path) -> Result<Vec<u8>> {
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let opened = age::Decryptor::new_buffered(ArmoredReader::new(&bytes[..]))
            .and_then(|d| d.decrypt(self.0.iter().map(|k| k as &dyn age::Identity)));
        let mut reader = match opened {
            Err(age::DecryptError::NoMatchingKeys) => {
                bail!("none of this machine's keys opens {}", path.display())
            }
            opened => opened.with_context(|| format!("decrypting {}", path.display()))?,
        };
        let mut plain = Vec::new();
        reader.read_to_end(&mut plain)?;
        Ok(plain)
    }
}

fn read_key(path: &Path) -> Result<Option<x25519::Identity>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let line = text
        .lines()
        .find(|line| line.starts_with("AGE-SECRET-KEY-"))
        .with_context(|| format!("{} holds no age key", path.display()))?;
    let key = line
        .parse()
        .map_err(|e: &str| anyhow!("{}: {e}", path.display()))?;
    Ok(Some(key))
}

pub fn opens(list: &Path, key: &Path) -> Result<bool> {
    let Some(key) = read_key(key)? else {
        return Ok(false);
    };
    let public = key.to_public().to_string();
    Ok(read(list)?.iter().any(|r| r.key == public))
}

pub fn encrypt(list: &Path, bytes: &[u8]) -> Result<Vec<u8>> {
    encrypt_for(&read(list)?, bytes).with_context(|| format!("encrypting for {}", list.display()))
}

fn encrypt_for(list: &[Recipient], bytes: &[u8]) -> Result<Vec<u8>> {
    let recipients = list
        .iter()
        .map(|r| r.key.parse().map_err(|e: &str| anyhow!("{}: {e}", r.name)))
        .collect::<Result<Vec<x25519::Recipient>>>()?;
    let encryptor =
        age::Encryptor::with_recipients(recipients.iter().map(|r| r as &dyn age::Recipient))?;
    let mut sealed = Vec::new();
    let mut writer =
        encryptor.wrap_output(ArmoredWriter::wrap_output(&mut sealed, Format::AsciiArmor)?)?;
    writer.write_all(bytes)?;
    writer.finish()?.finish()?;
    Ok(sealed)
}

#[derive(Debug, PartialEq)]
struct Recipient {
    name: String,
    key: String,
}

fn parse(text: &str) -> Result<Vec<Recipient>> {
    let mut list = Vec::new();
    let mut name = None;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(comment) = line.strip_prefix('#') {
            name = Some(comment.trim().to_owned());
        } else {
            let name = name
                .take()
                .with_context(|| format!("{line} has no `# name` above it"))?;
            list.push(Recipient {
                name,
                key: line.to_owned(),
            });
        }
    }
    Ok(list)
}

fn format(list: &[Recipient]) -> String {
    list.iter()
        .map(|r| format!("# {}\n{}\n", r.name, r.key))
        .collect::<Vec<_>>()
        .join("\n")
}

fn read(path: &Path) -> Result<Vec<Recipient>> {
    match fs::read_to_string(path) {
        Ok(text) => parse(&text).with_context(|| format!("parsing {}", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

pub fn key(fleet: &Fleet) -> Result<()> {
    let mut owners = vec![("root".to_owned(), None, fleet.recipients())];
    for member in &fleet.members {
        if let Some(user) = &member.user {
            owners.push((member.name.clone(), Some(user), member.recipients()));
        }
    }
    let mut untrusted = Vec::new();
    for (name, user, list) in owners {
        let path = user.map_or_else(|| PathBuf::from(ROOT_KEY), key_path);
        if read_key(&path)?.is_none() {
            make_key(&path, user)?;
        }
        let public = read_key(&path)?.expect("just made").to_public().to_string();
        println!("{name:6}  {public}");
        if !opens(&list, &path)? {
            untrusted.push(format!(
                "  sudo mothership trust {}/{name} <name> {public}",
                fleet.type_name
            ));
        }
    }
    if !untrusted.is_empty() {
        println!("Add them from a machine that opens the secrets, or here for the first one:");
        for line in untrusted {
            println!("{line}");
        }
    }
    Ok(())
}

fn make_key(path: &Path, owner: Option<&User>) -> Result<()> {
    let dir = path.parent().context("the key has no folder")?;
    match owner {
        Some(user) => user.create_dir_all(dir)?,
        None => fs::create_dir_all(dir)?,
    }
    let key = x25519::Identity::generate();
    let text = format!(
        "# public key: {}\n{}\n",
        key.to_public(),
        key.to_string().expose_secret()
    );
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .with_context(|| format!("writing {}", path.display()))?;
    if let Some(user) = owner {
        user.own(path)?;
    }
    Ok(())
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
    let type_dir = fleet.dir.join("types").join(type_name);
    if !type_dir.is_dir() {
        bail!("the fleet has no type {type_name}");
    }
    let (scope, list) = match scope {
        "root" => (Scope::Root, type_dir.join(RECIPIENTS)),
        user => {
            let dir = type_dir.join("users").join(user);
            if !dir.is_dir() {
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
    let target = target(fleet, text)?;
    key.parse::<x25519::Recipient>()
        .map_err(|e| anyhow!("{key}: {e}"))?;
    let mut list = read(&target.list)?;
    if let Some(known) = list.iter().find(|r| r.name == name || r.key == key) {
        bail!("{text} already holds {}", known.name);
    }
    list.push(Recipient {
        name: name.to_owned(),
        key: key.to_owned(),
    });
    let rewritten = rewrite(fleet, &target, &list)?;
    println!(
        "{name} opens the {} secrets of {text} now. Commit and push the fleet.",
        rewritten.len()
    );
    Ok(())
}

pub fn retire(fleet: &Fleet, text: &str, name: &str) -> Result<()> {
    let target = target(fleet, text)?;
    let mut list = read(&target.list)?;
    let before = list.len();
    list.retain(|r| r.name != name);
    if list.len() == before {
        bail!("{text} holds no {name}");
    }
    if list.is_empty() {
        bail!("{name} is the last key of {text}: trust another one first");
    }
    let rewritten = rewrite(fleet, &target, &list)?;
    println!("{name} is gone from {text}. Commit and push the fleet.");
    if !rewritten.is_empty() {
        println!("git history keeps the versions {name} opens, so change these:");
        for shown in rewritten {
            println!("  {shown}");
        }
    }
    Ok(())
}

fn rewrite(fleet: &Fleet, target: &Target, list: &[Recipient]) -> Result<Vec<String>> {
    let all = secrets_of(fleet, target)?;
    let sealed = all
        .values()
        .map(|source| encrypt_for(list, &fleet.keys().decrypt(source)?))
        .collect::<Result<Vec<_>>>()?;
    fs::write(&target.list, format(list))
        .with_context(|| format!("writing {}", target.list.display()))?;
    for (source, bytes) in all.values().zip(&sealed) {
        fs::write(source, bytes).with_context(|| format!("writing {}", source.display()))?;
    }
    Ok(all.into_keys().collect())
}

fn secrets_of(fleet: &Fleet, target: &Target) -> Result<BTreeMap<String, PathBuf>> {
    let type_dir = fleet.dir.join("types").join(&target.type_name);
    let mut all = BTreeMap::new();
    match &target.scope {
        Scope::Root => {
            let etc = type_dir.join("etc");
            for rel in walk::files(&etc)? {
                if let Some(clear) = rel.to_str().and_then(|r| r.strip_suffix(AGE)) {
                    all.insert(format!("/etc/{clear}"), etc.join(&rel));
                }
            }
        }
        Scope::User(user) => {
            for (target_path, source) in chezmoi::encrypted(fleet, &target.type_name, user)? {
                all.insert(format!("~/{}", target_path.display()), source);
            }
            let dir = type_dir.join("users").join(user).join(SECRETS);
            if dir.exists() {
                for rel in walk::files(&dir)? {
                    let source = dir.join(&rel);
                    let shown = source.strip_prefix(&fleet.dir)?.display().to_string();
                    all.insert(shown, source);
                }
            }
        }
    }
    Ok(all)
}

#[cfg(test)]
#[path = "../tests/secrets.rs"]
mod tests;
