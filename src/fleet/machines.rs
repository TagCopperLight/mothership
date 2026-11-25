use super::{SYSTEM_TOML, System, USERS, homes::Reader, read_toml, type_dir};
use crate::{
    secrets::Recipient,
    system::host,
    trust::{
        sign::{Reach, Signer},
        ssh,
    },
    util::{file, walk},
};
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use ssh_key::PublicKey;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub const MACHINES: &str = "machines";
const SUFFIX: &str = ".toml";
const EVERYTHING: &str = "everything";

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    #[serde(rename = "type")]
    pub device_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default, with = "signs", skip_serializing_if = "Option::is_none")]
    pub signs: Option<Reach>,
    #[serde(default, skip_serializing_if = "RootKeys::is_empty")]
    pub keys: RootKeys,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub users: BTreeMap<String, UserKeys>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct RootKeys {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<String>,
}

impl RootKeys {
    fn is_empty(&self) -> bool {
        self == &RootKeys::default()
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct UserKeys {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<String>,
}

mod signs {
    use super::EVERYTHING;
    use crate::trust::sign::Reach;
    use serde::{Deserialize, Deserializer, Serialize, Serializer, de, ser};

    pub fn serialize<S: Serializer>(reach: &Option<Reach>, s: S) -> Result<S::Ok, S::Error> {
        match reach {
            Some(Reach::Everything) => s.serialize_str(EVERYTHING),
            Some(Reach::Folders(folders)) => folders.serialize(s),
            None | Some(Reach::Versions) => Err(ser::Error::custom("signs names no folders")),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Reach>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Text {
            One(String),
            Folders(Vec<String>),
        }
        let words = match Text::deserialize(d)? {
            Text::One(word) if word == EVERYTHING => return Ok(Some(Reach::Everything)),
            Text::One(word) => {
                return Err(de::Error::custom(format!(
                    "signs is \"{EVERYTHING}\" or a list of folders, not \"{word}\""
                )));
            }
            Text::Folders(words) => words,
        };
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        match Reach::parse(&words).map_err(|e| de::Error::custom(format!("{e:#}")))? {
            Reach::Folders(folders) if !folders.is_empty() => Ok(Some(Reach::Folders(folders))),
            _ => Err(de::Error::custom(format!(
                "signs is \"{EVERYTHING}\" or a list of folders"
            ))),
        }
    }
}

impl Entry {
    pub fn new(device_type: &str, options: &[String]) -> Entry {
        Entry {
            device_type: device_type.to_owned(),
            options: options.to_vec(),
            signs: None,
            keys: RootKeys::default(),
            users: BTreeMap::new(),
        }
    }

    pub fn reaches(&self) -> impl Iterator<Item = Reach> + '_ {
        let probe = self.keys.probe.as_ref().map(|_| Reach::Versions);
        self.signs.clone().into_iter().chain(probe)
    }

    fn opens(&self, reader: &Reader) -> Option<&str> {
        if self.device_type != reader.type_name() {
            return None;
        }
        match reader {
            Reader::Root(_) => self.keys.age.as_deref(),
            Reader::Home(home) => self.users.get(&home.user)?.age.as_deref(),
        }
    }
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Machines(BTreeMap<String, Entry>);

pub fn path(fleet_dir: &Path, name: &str) -> PathBuf {
    fleet_dir.join(MACHINES).join(format!("{name}{SUFFIX}"))
}

pub fn named(rel: &Path) -> Option<&str> {
    let name = rel
        .strip_prefix(MACHINES)
        .ok()?
        .to_str()?
        .strip_suffix(SUFFIX)?;
    (!name.contains('/')).then_some(name)
}

pub fn probe_name(machine: &str) -> String {
    format!("{machine}/probe")
}

pub fn same_key(listed: Option<&str>, public: &str) -> bool {
    listed.is_some_and(|listed| {
        listed
            .split_whitespace()
            .take(2)
            .eq(public.split_whitespace().take(2))
    })
}

impl Machines {
    pub fn load(fleet_dir: &Path) -> Result<Machines> {
        let mut files = Vec::new();
        for path in walk::children(&fleet_dir.join(MACHINES))? {
            let rel = path.strip_prefix(fleet_dir)?.to_path_buf();
            let text = file::read_if_any(&path)?
                .with_context(|| format!("{} isn't a file", rel.display()))?;
            files.push((rel, text));
        }
        Machines::parse(files)
    }

    pub fn parse(files: Vec<(PathBuf, String)>) -> Result<Machines> {
        let mut machines = BTreeMap::new();
        for (rel, text) in files {
            let shown = rel.display();
            let name = named(&rel)
                .with_context(|| format!("{shown} isn't a machine's file, <name>{SUFFIX}"))?;
            host::check(name).with_context(|| format!("{shown}'s name"))?;
            let entry: Entry = toml::from_str(&text).with_context(|| format!("parsing {shown}"))?;
            if entry.keys.signing.is_some() != entry.signs.is_some() {
                bail!("{shown} needs both a signing key and `signs`, or neither");
            }
            machines.insert(name.to_owned(), entry);
        }
        Ok(Machines(machines))
    }

    pub fn check(&self, fleet_dir: &Path) -> Result<()> {
        for (name, entry) in &self.0 {
            let dir = type_dir(fleet_dir, &entry.device_type);
            if !dir.is_dir() {
                bail!(
                    "{name} is a {}, and the fleet has no such type",
                    entry.device_type
                );
            }
            if let Some(user) = entry
                .users
                .keys()
                .find(|u| !dir.join(USERS).join(u).is_dir())
            {
                bail!(
                    "{name} lists keys for {user}, and {} has no such user",
                    entry.device_type
                );
            }
        }
        Ok(())
    }

    pub fn check_options(fleet_dir: &Path, entry: &Entry) -> Result<()> {
        let system_path = type_dir(fleet_dir, &entry.device_type).join(SYSTEM_TOML);
        let system: System = read_toml(&system_path)?;
        if let Some(option) = entry
            .options
            .iter()
            .find(|o| !system.options.contains_key(*o))
        {
            bail!("{} has no option {option}", entry.device_type);
        }
        Ok(())
    }

    pub fn entries(&self) -> impl Iterator<Item = (&String, &Entry)> {
        self.0.iter()
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.0.get(name)
    }

    pub fn insert(&mut self, name: &str, entry: Entry) {
        self.0.insert(name.to_owned(), entry);
    }

    pub fn remove(&mut self, name: &str) -> Option<Entry> {
        self.0.remove(name)
    }

    pub fn write(&self, fleet_dir: &Path, name: &str) -> Result<()> {
        let path = path(fleet_dir, name);
        match self.0.get(name) {
            Some(entry) => file::write(&path, toml::to_string(entry)?.as_bytes(), 0o644),
            None => file::remove_if_found(&path).map(drop),
        }
        .with_context(|| format!("writing {}", path.display()))
    }

    pub fn signers(&self) -> Result<Vec<Signer>> {
        let mut signers = Vec::new();
        for (name, entry) in &self.0 {
            let keys = [
                (name.clone(), &entry.keys.signing, entry.signs.clone()),
                (probe_name(name), &entry.keys.probe, Some(Reach::Versions)),
            ];
            for (name, key, reach) in keys {
                if let (Some(key), Some(reach)) = (key, reach) {
                    let key = PublicKey::from_openssh(key)
                        .map_err(|e| anyhow!("{name}'s signing key: {e}"))?;
                    signers.push(Signer { name, key, reach });
                }
            }
        }
        Ok(signers)
    }

    pub fn sealed(&self, readers: &[Reader], recovery: Option<&Recipient>) -> Vec<Recipient> {
        let mut list = self.recipients_of(readers);
        if let Some(recovery) = recovery.filter(|r| !list.iter().any(|known| known.key == r.key)) {
            list.push(recovery.clone());
        }
        list
    }

    pub fn recipients(&self, reader: &Reader) -> Vec<Recipient> {
        self.recipients_of(std::slice::from_ref(reader))
    }

    pub fn recipients_of(&self, readers: &[Reader]) -> Vec<Recipient> {
        let mut all: Vec<Recipient> = Vec::new();
        for reader in readers {
            let folder = reader.dir(Path::new(""));
            for (name, entry) in &self.0 {
                let signs = entry.signs.as_ref().filter(|reach| reach.covers(&folder));
                let root = signs.and(entry.keys.age.as_deref());
                for key in entry.opens(reader).into_iter().chain(root) {
                    if !all.iter().any(|known| known.key == key) {
                        all.push(Recipient {
                            name: name.clone(),
                            key: key.to_owned(),
                        });
                    }
                }
            }
        }
        all
    }

    pub fn logins(&self, type_name: &str, user: &str) -> Result<Vec<String>> {
        let mut lines = Vec::new();
        for (name, entry) in &self.0 {
            if entry.device_type != type_name {
                continue;
            }
            if let Some(key) = entry.users.get(user).and_then(|keys| keys.ssh.as_deref()) {
                lines.push(format!("{} {name}/{user}", ssh::normalize(key)?));
            }
        }
        Ok(lines)
    }
}

#[cfg(test)]
#[path = "../../tests/fleet/machines.rs"]
mod tests;
