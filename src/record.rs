use crate::{
    fleet::{Fleet, Tools},
    package::{self, Db},
    revert::{Owned, Revert},
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

    pub fn fleet_changed(&self, revert: &Revert, user: Option<&str>, db: &Db) -> Result<bool> {
        let empty = UserRecord::default();
        let mine = user.and_then(|u| self.users.get(u)).unwrap_or(&empty);
        Ok(match revert {
            Revert::Install(name) => !self.packages.contains(name),
            Revert::Uninstall(name) => self.packages.contains(name),
            Revert::CreateUser | Revert::InitChezmoi => true,
            Revert::Linger(linger) => mine.linger != *linger,
            Revert::InstallTool { manager, tool } | Revert::ReinstallTool { manager, tool } => {
                mine.tools.find(*manager, tool.name()) != Some(tool)
            }
            Revert::UninstallTool { manager, name } => mine.tools.find(*manager, name).is_some(),
            Revert::RunInstaller(name) => !mine.installers.contains(name),
            Revert::Join(group) => !mine.groups.contains(group),
            Revert::Leave(group) => mine.groups.contains(group),
            Revert::Shell(shell) => mine.shell.as_ref() != Some(shell),
            Revert::Enable { unit, user: false } => !self.services.contains(unit),
            Revert::Disable { unit, user: false } => self.services.contains(unit),
            Revert::Enable { unit, .. } => !mine.user_services.contains(unit),
            Revert::Disable { unit, .. } => mine.user_services.contains(unit),
            Revert::WriteEtc(path) => match hash(path)? {
                None => !self.etc.contains_key(path),
                Some(actual) => {
                    self.etc.get(path) == Some(&actual) || shipped(db, path) == Some(&actual)
                }
            },
            Revert::RestoreEtc { path, .. } => self.unchanged(path)?,
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

    fn unchanged(&self, path: &Path) -> Result<bool> {
        Ok(self.etc.contains_key(path) && hash(path)?.as_ref() == self.etc.get(path))
    }

    /// What the fleet says now, except for the `skipped` changes, which keep what the record
    /// said before so the next apply still brings them.
    pub fn after(
        fleet: &Fleet,
        before: &Record,
        etc_root: &Path,
        skipped: &[Owned],
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
                for (_, revert) in skipped.iter().filter(|(owner, _)| *owner == Some(i)) {
                    match revert {
                        Revert::Linger(_) => record.linger = old.linger,
                        Revert::Enable { unit, .. } | Revert::Disable { unit, .. } => {
                            if old.user_services.contains(unit) {
                                record.user_services.insert(unit.clone());
                            } else {
                                record.user_services.remove(unit);
                            }
                        }
                        _ => unreachable!("apply skips only what needs_service_manager names"),
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
