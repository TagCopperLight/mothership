use crate::{
    fleet::{Fleet, Tools},
    package::{self, Db},
    revert::{Account, Etc, Machine, Revert, Service, ToolChange, Unit, User},
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

#[derive(Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Record {
    packages: BTreeSet<String>,
    services: BTreeSet<String>,
    etc: BTreeMap<PathBuf, String>,
    users: BTreeMap<String, UserRecord>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default, deny_unknown_fields)]
struct UserRecord {
    groups: BTreeSet<String>,
    shell: Option<String>,
    linger: bool,
    user_services: BTreeSet<String>,
    tools: Tools,
    installers: BTreeSet<String>,
}

impl Record {
    pub fn load(path: &Path) -> Result<Self> {
        walk::read_json(path)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        walk::write_json(path, self)
    }

    pub fn fleet_changed(&self, revert: &Revert, user: Option<&str>, db: &Db) -> Result<bool> {
        let change = match revert {
            Revert::Machine(change) => return self.machine_changed(change, db),
            Revert::User(change) => change,
        };
        let empty = UserRecord::default();
        let mine = user.and_then(|u| self.users.get(u)).unwrap_or(&empty);
        Ok(match change {
            User::Create | User::InitChezmoi => true,
            User::Service(Service::Linger(linger)) => mine.linger != *linger,
            User::Service(Service::Unit(Unit::Enable(unit))) => !mine.user_services.contains(unit),
            User::Service(Service::Unit(Unit::Disable(unit))) => mine.user_services.contains(unit),
            User::Tool(
                ToolChange::Install { manager, tool } | ToolChange::Reinstall { manager, tool },
            ) => mine.tools.find(*manager, tool.name()) != Some(tool),
            User::Tool(ToolChange::Uninstall { manager, name }) => {
                mine.tools.find(*manager, name).is_some()
            }
            User::RunInstaller(name) => !mine.installers.contains(name),
            User::Account(Account::Join(group)) => !mine.groups.contains(group),
            User::Account(Account::Leave(group)) => mine.groups.contains(group),
            User::Account(Account::Shell(shell)) => mine.shell.as_ref() != Some(shell),
            User::ApplyFile { edited, .. } => !edited,
        })
    }

    fn machine_changed(&self, change: &Machine, db: &Db) -> Result<bool> {
        Ok(match change {
            Machine::Install(name) => !self.packages.contains(name),
            Machine::Uninstall(name) => self.packages.contains(name),
            Machine::Unit(Unit::Enable(unit)) => !self.services.contains(unit),
            Machine::Unit(Unit::Disable(unit)) => self.services.contains(unit),
            Machine::Etc(Etc::Write(path)) => match hash(path)? {
                None => !self.etc.contains_key(path),
                Some(actual) => {
                    self.etc.get(path) == Some(&actual) || shipped(db, path) == Some(&actual)
                }
            },
            Machine::Etc(Etc::Restore { path, .. }) => self.unchanged(path)?,
            Machine::Etc(Etc::Delete(path)) if path.is_dir() => {
                self.etc.keys().any(|written| written.starts_with(path))
                    && walk::files(path)?
                        .iter()
                        .map(|file| self.unchanged(&path.join(file)))
                        .collect::<Result<Vec<_>>>()?
                        .into_iter()
                        .all(|unchanged| unchanged)
            }
            Machine::Etc(Etc::Delete(path)) => self.unchanged(path)?,
        })
    }

    pub fn reverted(&mut self, revert: &Revert) -> Result<()> {
        let Revert::Machine(Machine::Etc(change)) = revert else {
            return Ok(());
        };
        match change {
            Etc::Write(path) => {
                let actual = hash(path)?.context("the fleet's file was just written")?;
                self.etc.insert(path.clone(), actual);
            }
            Etc::Restore { path, .. } | Etc::Delete(path) => {
                self.etc.retain(|written, _| !written.starts_with(path))
            }
        }
        Ok(())
    }

    fn unchanged(&self, path: &Path) -> Result<bool> {
        Ok(self.etc.contains_key(path) && hash(path)?.as_ref() == self.etc.get(path))
    }

    pub fn after(
        fleet: &Fleet,
        before: &Record,
        etc_root: &Path,
        skipped: &[(usize, &Service)],
    ) -> Result<Record> {
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
        let empty = UserRecord::default();
        let users = fleet
            .members
            .iter()
            .enumerate()
            .filter_map(|(i, member)| {
                if member.user.is_none() {
                    let old = before.users.get(&member.name)?;
                    return Some((member.name.clone(), old.clone()));
                }
                let config = &member.config;
                let mut record = UserRecord {
                    groups: config.groups.iter().cloned().collect(),
                    shell: config.shell.clone(),
                    linger: config.linger,
                    user_services: config.user_services.iter().cloned().collect(),
                    tools: config.tools.clone(),
                    installers: config.installers.iter().map(|i| i.name.clone()).collect(),
                };
                let old = before.users.get(&member.name).unwrap_or(&empty);
                for (_, service) in skipped.iter().filter(|(owner, _)| *owner == i) {
                    match service {
                        Service::Linger(_) => record.linger = old.linger,
                        Service::Unit(Unit::Enable(unit) | Unit::Disable(unit)) => {
                            if old.user_services.contains(unit) {
                                record.user_services.insert(unit.clone());
                            } else {
                                record.user_services.remove(unit);
                            }
                        }
                    }
                }
                Some((member.name.clone(), record))
            })
            .collect();
        Ok(Record {
            packages: fleet.packages(),
            services: fleet.system.services.iter().cloned().collect(),
            etc,
            users,
        })
    }
}

fn wanted(fleet: &Fleet, rel: &Path) -> Result<Option<String>> {
    if !fleet.secret_etc.contains(rel) {
        return hash(&fleet.etc_source(rel));
    }
    if !fleet.etc_open() {
        return Ok(None);
    }
    Ok(Some(package::md5(&fleet.etc_contents(rel)?)))
}

fn hash(path: &Path) -> Result<Option<String>> {
    let bytes = match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_symlink() => fs::read_link(path)?.into_os_string().into_encoded_bytes(),
        Ok(_) => fs::read(path)?,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    Ok(Some(package::md5(&bytes)))
}

fn shipped<'a>(db: &'a Db, path: &Path) -> Option<&'a String> {
    db.packages
        .iter()
        .flat_map(|package| &package.backups)
        .find(|backup| backup.path == path)
        .map(|backup| &backup.md5)
}

#[cfg(test)]
#[path = "../tests/record.rs"]
mod tests;
