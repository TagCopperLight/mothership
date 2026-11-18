use crate::{
    cmd,
    fleet::{Fleet, tilde},
    user,
};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

pub const RECIPIENTS: &str = "recipients.txt";
/// What marks a file of the type's etc/ as encrypted, as chezmoi marks its own.
pub const AGE: &str = ".age";

/// chezmoi does every encryption with its own age config, so the secrets in ~ and in /etc open
/// with the same key and go to the same recipients.
pub struct Secrets {
    pub key: PathBuf,
    pub recipients: PathBuf,
    /// Whether this machine's key is among the type's recipients.
    pub open: bool,
}

impl Secrets {
    /// `config` is `chezmoi dump-config`.
    pub fn new(config: &serde_json::Value, type_dir: &Path) -> Result<Self> {
        let recipients = type_dir.join(RECIPIENTS);
        let age = &config["age"];
        let path = |name: &str| {
            age[name]
                .as_str()
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
        };
        let (Some(key), true, true) = (
            path("identity"),
            config["encryption"] == "age",
            path("recipientsFile").as_ref() == Some(&recipients),
        ) else {
            bail!(
                "chezmoi's config must set encryption = \"age\", age.identity and \
                 age.recipientsFile = \"{}\"",
                recipients.display()
            );
        };
        let open = match public_key(&key)? {
            Some(public) => read(&recipients)?.iter().any(|r| r.key == public),
            None => false,
        };
        Ok(Secrets {
            key,
            recipients,
            open,
        })
    }

    pub fn check_open(&self) -> Result<()> {
        if !self.open {
            bail!(
                "this machine's key isn't in {}: `mothership trust` it first",
                tilde(&self.recipients)
            );
        }
        Ok(())
    }

    pub fn encrypt(&self, bytes: &[u8]) -> Result<Vec<u8>> {
        self.check_open()?;
        cmd::piped("chezmoi", &["encrypt"], bytes)
    }

    pub fn decrypt(&self, path: &Path) -> Result<Vec<u8>> {
        cmd::bytes("chezmoi", &["decrypt", &path.to_string_lossy()])
    }
}

/// From the comment age's key generators write above the key.
fn public_key(key: &Path) -> Result<Option<String>> {
    let text = match fs::read_to_string(key) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", key.display())),
    };
    let public = text
        .lines()
        .find_map(|line| line.strip_prefix("# public key: "))
        .with_context(|| format!("{} has no `# public key:` line", key.display()))?;
    Ok(Some(public.trim().to_owned()))
}

#[derive(Debug, PartialEq)]
struct Recipient {
    name: String,
    key: String,
}

/// age's recipients format, with each key named by the comment above it.
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

/// Makes this machine's key when it has none, and shows its public half.
pub fn key() -> Result<()> {
    let fleet = Fleet::load()?;
    let secrets = fleet.secrets();
    if public_key(&secrets.key)?.is_none() {
        user::create_dir_all(secrets.key.parent().context("the key has no folder")?)?;
        cmd::bytes(
            "chezmoi",
            &["age-keygen", "--output", &secrets.key.to_string_lossy()],
        )?;
    }
    let public = public_key(&secrets.key)?.expect("just made");
    println!("{public}");
    if !secrets.open {
        println!(
            "Add it from a machine that opens the secrets, or here for the first one:\n  \
             mothership trust <name> {public}"
        );
    }
    Ok(())
}

pub fn trust(name: &str, key: &str) -> Result<()> {
    let fleet = Fleet::load()?;
    let mut list = read(&fleet.secrets().recipients)?;
    if let Some(known) = list.iter().find(|r| r.name == name || r.key == key) {
        bail!("{} already holds {}", RECIPIENTS, known.name);
    }
    list.push(Recipient {
        name: name.to_owned(),
        key: key.to_owned(),
    });
    let rewritten = rewrite(&fleet, &list)?;
    println!(
        "{name} opens the type's {} secrets now. Commit and push the fleet.",
        rewritten.len()
    );
    Ok(())
}

pub fn retire(name: &str) -> Result<()> {
    let fleet = Fleet::load()?;
    let mut list = read(&fleet.secrets().recipients)?;
    let before = list.len();
    list.retain(|r| r.name != name);
    if list.len() == before {
        bail!("{} holds no {name}", RECIPIENTS);
    }
    let rewritten = rewrite(&fleet, &list)?;
    println!("{name} is gone. Commit and push the fleet.");
    if !rewritten.is_empty() {
        println!("git history keeps the versions {name} opens, so change these:");
        for target in rewritten {
            println!("  {target}");
        }
    }
    Ok(())
}

/// Writes the recipients and encrypts every secret of the type for them. Returns the secrets'
/// paths on the machine. Nothing changes when one fails.
fn rewrite(fleet: &Fleet, list: &[Recipient]) -> Result<Vec<String>> {
    let secrets = fleet.secrets();
    // chezmoi reads the key even to encrypt, which only needs the recipients.
    if public_key(&secrets.key)?.is_none() {
        bail!("this machine has no key yet: make it with `mothership key` first");
    }
    let all = type_secrets(fleet)?;
    if !all.is_empty() {
        secrets.check_open()?;
    }
    let plain = all
        .values()
        .map(|source| secrets.decrypt(source))
        .collect::<Result<Vec<_>>>()?;
    let old = fs::read(&secrets.recipients).ok();
    fs::write(&secrets.recipients, format(list))
        .with_context(|| format!("writing {}", secrets.recipients.display()))?;
    user::own(&secrets.recipients)?;
    // The empty one checks the keys when the type has no secret yet.
    let sealed = std::iter::once(&[][..])
        .chain(plain.iter().map(Vec::as_slice))
        .map(|bytes| cmd::piped("chezmoi", &["encrypt"], bytes))
        .collect::<Result<Vec<_>>>();
    let sealed = match sealed {
        Ok(sealed) => sealed,
        Err(e) => {
            match old {
                Some(old) => fs::write(&secrets.recipients, old)?,
                None => fs::remove_file(&secrets.recipients)?,
            }
            return Err(e);
        }
    };
    for (source, bytes) in all.values().zip(&sealed[1..]) {
        fs::write(source, bytes).with_context(|| format!("writing {}", source.display()))?;
    }
    Ok(all.into_keys().collect())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Managed {
    absolute: PathBuf,
    source_absolute: PathBuf,
}

/// Each secret's path on the machine, and its encrypted file in the fleet. The type's etc/ counts
/// whole, blocks this machine lacks included.
fn type_secrets(fleet: &Fleet) -> Result<BTreeMap<String, PathBuf>> {
    let args = [
        "managed",
        "--include",
        "encrypted",
        "--path-style",
        "all",
        "--format",
        "json",
    ];
    let home: BTreeMap<String, Managed> = serde_json::from_str(&cmd::stdout("chezmoi", &args)?)?;
    let mut all: BTreeMap<String, PathBuf> = home
        .into_values()
        .map(|m| (tilde(&m.absolute), m.source_absolute))
        .collect();
    all.extend(fleet.secret_etc.iter().map(|rel| {
        (
            Path::new("/etc").join(rel).display().to_string(),
            fleet.etc_source(rel),
        )
    }));
    Ok(all)
}

#[cfg(test)]
#[path = "../tests/secrets.rs"]
mod tests;
