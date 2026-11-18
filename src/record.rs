use crate::{
    fleet::{Fleet, Manager, Tool, Tools},
    pacman::{self, Db},
    revert::Revert,
    walk,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

pub const PATH: &str = "/var/lib/mothership/applied.json";

/// What the fleet declared at the last apply. A difference with the machine then comes either
/// from the fleet changing since, which apply follows, or from the machine changing since,
/// which waits for review.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Record {
    packages: BTreeSet<String>,
    groups: BTreeSet<String>,
    shell: Option<String>,
    services: BTreeSet<String>,
    user_services: BTreeSet<String>,
    tools: Tools,
    installers: BTreeSet<String>,
    /// The MD5 of each /etc file as apply last left it.
    etc: BTreeMap<PathBuf, String>,
}

impl Record {
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => {
                serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(Record::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        fs::create_dir_all(path.parent().expect("has a parent"))?;
        fs::write(path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// A file that still matches what apply wrote, or what its package shipped, wasn't edited.
    pub fn fleet_changed(&self, revert: &Revert, db: &Db) -> Result<bool> {
        Ok(match revert {
            Revert::Install(name) => !self.packages.contains(name),
            Revert::Uninstall(name) => self.packages.contains(name),
            Revert::InstallTool { manager, tool } | Revert::ReinstallTool { manager, tool } => {
                self.tool(*manager, tool.name()) != Some(tool)
            }
            Revert::UninstallTool { manager, name } => self.tool(*manager, name).is_some(),
            Revert::RunInstaller(name) => !self.installers.contains(name),
            Revert::Join(group) => !self.groups.contains(group),
            Revert::Leave(group) => self.groups.contains(group),
            Revert::Shell(shell) => self.shell.as_ref() != Some(shell),
            Revert::Enable { unit, user } => !self.units(*user).contains(unit),
            Revert::Disable { unit, user } => self.units(*user).contains(unit),
            Revert::WriteEtc(path) => match hash(path)? {
                None => !self.etc.contains_key(path),
                Some(actual) => {
                    self.etc.get(path) == Some(&actual) || shipped(db, path) == Some(&actual)
                }
            },
            Revert::RestoreEtc { path, .. } => self.unchanged(path)?,
            // Folders apply never wrote into, root's secrets among them, stay unread.
            Revert::DeleteEtc(path) if path.is_dir() => {
                self.etc.keys().any(|written| written.starts_with(path))
                    && walk::files(path)?
                        .iter()
                        .map(|file| self.unchanged(&path.join(file)))
                        .collect::<Result<Vec<_>>>()?
                        .into_iter()
                        .all(|unchanged| unchanged)
            }
            Revert::DeleteEtc(path) => self.unchanged(path)?,
            Revert::ApplyFile { edited, .. } => !edited,
        })
    }

    /// Review's reverts write /etc too, and apply must know those files as its own afterwards.
    pub fn reverted(&mut self, revert: &Revert) -> Result<()> {
        match revert {
            Revert::WriteEtc(path) => {
                let actual = hash(path)?.context("the fleet's file was just written")?;
                self.etc.insert(path.clone(), actual);
            }
            Revert::RestoreEtc { path, .. } | Revert::DeleteEtc(path) => {
                self.etc.retain(|written, _| !written.starts_with(path))
            }
            _ => {}
        }
        Ok(())
    }

    fn tool(&self, manager: Manager, name: &str) -> Option<&Tool> {
        self.tools.find(manager, name)
    }

    fn units(&self, user: bool) -> &BTreeSet<String> {
        if user {
            &self.user_services
        } else {
            &self.services
        }
    }

    fn unchanged(&self, path: &Path) -> Result<bool> {
        Ok(self.etc.contains_key(path) && hash(path)?.as_ref() == self.etc.get(path))
    }

    /// An /etc file left edited keeps its old entry, so it stays an edit.
    pub fn after(fleet: &Fleet, before: &Record, etc_root: &Path) -> Result<Record> {
        let mut etc = BTreeMap::new();
        for rel in &fleet.etc {
            let path = etc_root.join(rel);
            let wanted = wanted(fleet, rel)?;
            let entry = if hash(&path)? == wanted {
                wanted
            } else {
                before.etc.get(&path).cloned()
            };
            if let Some(entry) = entry {
                etc.insert(path, entry);
            }
        }
        let system = &fleet.system;
        Ok(Record {
            packages: fleet.packages(),
            groups: system.groups.iter().cloned().collect(),
            shell: system.shell.clone(),
            services: system.services.iter().cloned().collect(),
            user_services: system.user_services.iter().cloned().collect(),
            tools: system.tools.clone(),
            installers: system.installers.iter().map(|i| i.name.clone()).collect(),
            etc,
        })
    }
}

/// Apply never writes a secret this machine can't open.
fn wanted(fleet: &Fleet, rel: &Path) -> Result<Option<String>> {
    if !fleet.secret_etc.contains(rel) {
        return hash(&fleet.etc_source(rel));
    }
    if !fleet.secrets().open {
        return Ok(None);
    }
    Ok(Some(pacman::md5(&fleet.etc_contents(rel)?)))
}

/// A link's hash is its target's path.
fn hash(path: &Path) -> Result<Option<String>> {
    let bytes = match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_symlink() => fs::read_link(path)?.into_os_string().into_encoded_bytes(),
        Ok(_) => fs::read(path)?,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    Ok(Some(pacman::md5(&bytes)))
}

fn shipped<'a>(db: &'a Db, path: &Path) -> Option<&'a String> {
    db.packages
        .iter()
        .flat_map(|package| &package.backups)
        .find(|(backup, _)| backup == path)
        .map(|(_, md5)| md5)
}

#[cfg(test)]
#[path = "../tests/record.rs"]
mod tests;
