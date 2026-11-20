use crate::{file, user::User};
use age::{
    armor::{ArmoredReader, ArmoredWriter, Format},
    secrecy::ExposeSecret,
    x25519,
};
use anyhow::{Context, Result, anyhow, bail};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

pub const RECIPIENTS: &str = "recipients.txt";
pub const ROOT_KEY: &str = "/etc/mothership/key.txt";
pub const AGE: &str = ".age";

pub fn key_path(user: &User) -> PathBuf {
    user.home.join(".config/mothership/key.txt")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Storage {
    Clear,
    Encrypted,
}

#[derive(Clone, Default)]
pub struct Keys(Vec<x25519::Identity>);

impl Keys {
    pub fn push(&mut self, key: x25519::Identity) {
        self.0.push(key);
    }

    pub fn add(&mut self, secret: &str) -> Result<String> {
        let key: x25519::Identity = secret
            .trim()
            .parse()
            .map_err(|e: &str| anyhow!("that isn't an age secret key: {e}"))?;
        let public = key.to_public().to_string();
        self.0.push(key);
        Ok(public)
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

pub fn public(key: &x25519::Identity) -> String {
    key.to_public().to_string()
}

fn parse_key(text: &[u8], shown: &Path) -> Result<x25519::Identity> {
    String::from_utf8_lossy(text)
        .lines()
        .find(|line| line.starts_with("AGE-SECRET-KEY-"))
        .with_context(|| format!("{} holds no age key", shown.display()))?
        .parse()
        .map_err(|e: &str| anyhow!("{}: {e}", shown.display()))
}

pub fn root_key() -> Result<Option<x25519::Identity>> {
    let path = Path::new(ROOT_KEY);
    let Some(meta) = file::if_found(fs::symlink_metadata(path))
        .with_context(|| format!("reading {ROOT_KEY}"))?
    else {
        return Ok(None);
    };
    if !meta.is_file() || meta.uid() != 0 || meta.mode() & 0o077 != 0 {
        bail!(
            "{ROOT_KEY} must be a file only root can read: `chown root: {ROOT_KEY} && chmod 600 {ROOT_KEY}`"
        );
    }
    let bytes = file::read(path)?.with_context(|| format!("{ROOT_KEY} vanished"))?;
    parse_key(&bytes, path).map(Some)
}

pub fn user_key(user: &User) -> Result<Option<x25519::Identity>> {
    let path = key_path(user);
    match user.read(&path)? {
        Some(bytes) => parse_key(&bytes, &path).map(Some),
        None => Ok(None),
    }
}

pub fn encrypt_for(list: &[Recipient], bytes: &[u8]) -> Result<Vec<u8>> {
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

#[derive(Debug, PartialEq, Clone)]
pub struct Recipient {
    pub name: String,
    pub key: String,
}

pub fn parse(text: &str) -> Result<Vec<Recipient>> {
    let mut list: Vec<Recipient> = Vec::new();
    let mut name = None;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(comment) = line.strip_prefix('#') {
            if let Some(unused) = name.replace(comment.trim().to_owned()) {
                bail!("# {unused} names no key");
            }
            continue;
        }
        let name = name
            .take()
            .with_context(|| format!("{line} has no `# name` above it"))?;
        if let Some(known) = list.iter().find(|r| r.name == name || r.key == line) {
            bail!("{} is in the list twice", known.name);
        }
        list.push(Recipient {
            name,
            key: line.to_owned(),
        });
    }
    if let Some(unused) = name {
        bail!("# {unused} names no key");
    }
    Ok(list)
}

pub fn format(list: &[Recipient]) -> String {
    list.iter()
        .map(|r| format!("# {}\n{}\n", r.name, r.key))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn names(list: &Path) -> Result<Vec<String>> {
    Ok(read(list)?.into_iter().map(|r| r.name).collect())
}

pub fn holds(list: &Path, public: &str) -> Result<bool> {
    Ok(read(list)?.iter().any(|r| r.key == public))
}

pub fn read(path: &Path) -> Result<Vec<Recipient>> {
    match file::read_if_any(path)? {
        Some(text) => parse(&text).with_context(|| format!("parsing {}", path.display())),
        None => Ok(Vec::new()),
    }
}

pub fn make_key(path: &Path) -> Result<x25519::Identity> {
    fs::create_dir_all(path.parent().context("the key has no folder")?)?;
    let key = x25519::Identity::generate();
    write_key(path, &key)?;
    Ok(key)
}

pub fn write_key(path: &Path, key: &x25519::Identity) -> Result<()> {
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
        .with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
#[path = "../tests/secrets.rs"]
mod tests;
