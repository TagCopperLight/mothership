use crate::{
    fleet::Fleet,
    pacman::{self, Db},
    prompt::Prompt,
    revert::{self, Revert},
    scan, user, walk,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

const RECORD: &str = "/var/lib/mothership/applied.json";

pub fn run() -> Result<()> {
    let fleet = Fleet::load()?;
    let db = Db::load()?;
    let record_path = Path::new(RECORD);
    let record = Record::load(record_path)?;
    let mut items = scan::system(&fleet, &db)?;
    items.extend(scan::files::scan()?);

    let (mut todo, mut edited, mut left) = (Vec::new(), Vec::new(), 0);
    for item in &items {
        match &item.revert {
            Some(revert) if record.fleet_changed(revert, &db)? => todo.push(item),
            _ if fleet.ignore.matches(item) => {}
            Some(Revert::WriteEtc(_) | Revert::ApplyFile { .. }) => edited.push(item),
            _ => left += 1,
        }
    }

    scan::print(&todo, |item| {
        item.revert.as_ref().expect("filtered above").describe()
    });
    if !edited.is_empty() {
        println!("Left alone, since they were edited on this machine:");
        for item in &edited {
            println!("  {}", item.subject);
        }
        println!();
    }
    if left > 0 {
        println!("{left} other differences wait for `mothership review`.");
    }
    if todo.is_empty() {
        println!("Nothing to apply.");
    } else if !user::is_root() {
        println!("Run it with sudo to apply. Without root, unreadable /etc files are left out.");
        return Ok(());
    } else if Prompt::new().confirm("Apply")? {
        let reverts: Vec<&Revert> = todo.iter().filter_map(|i| i.revert.as_ref()).collect();
        revert::run(&fleet, &reverts)?;
    } else {
        return Ok(());
    }
    if user::is_root() {
        Record::after(&fleet, &record, Path::new("/etc"))?.save(record_path)?;
    }
    Ok(())
}

/// What the fleet declared at the last apply. A difference with the machine then comes either
/// from the fleet changing since, which apply follows, or from the machine changing since,
/// which waits for review.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Record {
    packages: BTreeSet<String>,
    groups: BTreeSet<String>,
    shell: Option<String>,
    services: BTreeSet<String>,
    user_services: BTreeSet<String>,
    /// The MD5 of each /etc file as apply last left it.
    etc: BTreeMap<PathBuf, String>,
}

impl Record {
    fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => {
                serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(Record::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    fn save(&self, path: &Path) -> Result<()> {
        fs::create_dir_all(path.parent().expect("has a parent"))?;
        fs::write(path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// A file that still matches what apply wrote, or what its package shipped, wasn't edited.
    fn fleet_changed(&self, revert: &Revert, db: &Db) -> Result<bool> {
        Ok(match revert {
            Revert::Install(name) => !self.packages.contains(name),
            Revert::Uninstall(name) => self.packages.contains(name),
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
    fn after(fleet: &Fleet, before: &Record, etc_root: &Path) -> Result<Record> {
        let mut etc = BTreeMap::new();
        let etc_dir = fleet.etc_dir();
        for rel in walk::files(&etc_dir)? {
            let path = etc_root.join(&rel);
            let wanted = hash(&etc_dir.join(&rel))?;
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
            packages: fleet.packages()?,
            groups: system.groups.iter().cloned().collect(),
            shell: system.shell.clone(),
            services: system.services.iter().cloned().collect(),
            user_services: system.user_services.iter().cloned().collect(),
            etc,
        })
    }
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
#[path = "../tests/apply.rs"]
mod tests;
