use crate::{
    fleet::{Fleet, Tools},
    package::Db,
    sync::revert::{Etc, Machine, Revert, Unit, User},
    util::file,
    util::hash,
    util::walk,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const PATH: &str = "/var/lib/mothership/applied.json";

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
    pub fn load() -> Result<Self> {
        file::read_json(Path::new(PATH))
    }

    pub fn save(&self) -> Result<()> {
        file::write_json(Path::new(PATH), self)
    }

    pub fn fleet_changed(&self, revert: &Revert, user: Option<&str>, db: &Db) -> Result<bool> {
        let change = match revert {
            Revert::Machine(change) => return self.machine_changed(change, db),
            Revert::User(change) => change,
        };
        let empty = UserRecord::default();
        let mine = user.and_then(|u| self.users.get(u)).unwrap_or(&empty);
        Ok(match change {
            User::InitChezmoi => true,
            User::Linger(linger) => mine.linger != *linger,
            User::Unit(Unit::Enable(unit)) => !mine.user_services.contains(unit),
            User::Unit(Unit::Disable(unit)) => mine.user_services.contains(unit),
            User::InstallTool { manager, tool } | User::ReinstallTool { manager, tool } => {
                mine.tools.find(*manager, tool.name()) != Some(tool)
            }
            User::UninstallTool { manager, name } => mine.tools.find(*manager, name).is_some(),
            User::RunInstaller(name) => !mine.installers.contains(name),
            User::Join(group) => !mine.groups.contains(group),
            User::Leave(group) => mine.groups.contains(group),
            User::Shell(shell) => mine.shell.as_ref() != Some(shell),
            User::ApplyFile { edited, .. } => !edited,
            User::ResetDconf(_) => false,
        })
    }

    fn machine_changed(&self, change: &Machine, db: &Db) -> Result<bool> {
        Ok(match change {
            Machine::Install(name) => !self.packages.contains(name),
            Machine::Uninstall(name) => self.packages.contains(name),
            Machine::CreateUser { .. } => true,
            Machine::Container(_) => false,
            Machine::Unit(Unit::Enable(unit)) => !self.services.contains(unit),
            Machine::Unit(Unit::Disable(unit)) => self.services.contains(unit),
            Machine::Etc(Etc::Write(path)) => match hash(path)? {
                None => !self.etc.contains_key(path),
                Some(actual) => match self.etc.get(path) {
                    Some(written) => *written == actual,
                    None => shipped(db, path) == Some(&actual),
                },
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
        skipped: &[(usize, &User)],
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
        let missing = fleet.missing().filter_map(|profile| {
            let old = before.users.get(&profile.name)?;
            Some((profile.name.clone(), old.clone()))
        });
        let users = fleet
            .members
            .iter()
            .enumerate()
            .map(|(i, member)| {
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
                for (_, change) in skipped.iter().filter(|(owner, _)| *owner == i) {
                    match change {
                        User::Linger(_) => record.linger = old.linger,
                        User::Unit(Unit::Enable(unit) | Unit::Disable(unit)) => {
                            if old.user_services.contains(unit) {
                                record.user_services.insert(unit.clone());
                            } else {
                                record.user_services.remove(unit);
                            }
                        }
                        _ => unreachable!("only service changes are skipped"),
                    }
                }
                (member.name.clone(), record)
            })
            .chain(missing)
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
    Ok(Some(hash::md5(&fleet.etc_contents(rel)?)))
}

fn hash(path: &Path) -> Result<Option<String>> {
    let meta = file::if_found(fs::symlink_metadata(path))
        .with_context(|| format!("reading {}", path.display()))?;
    let bytes = match meta {
        Some(meta) if meta.is_symlink() => {
            fs::read_link(path)?.into_os_string().into_encoded_bytes()
        }
        Some(_) => fs::read(path)?,
        None => return Ok(None),
    };
    Ok(Some(hash::md5(&bytes)))
}

fn shipped<'a>(db: &'a Db, path: &Path) -> Option<&'a String> {
    db.packages
        .iter()
        .flat_map(|package| &package.backups)
        .find(|backup| backup.path == path)
        .map(|backup| &backup.md5)
}

#[cfg(test)]
#[path = "../../tests/sync/record.rs"]
mod tests;
