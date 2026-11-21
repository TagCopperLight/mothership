use super::services;
use crate::item::{Change, Item, Secret};
use crate::kind::Kind;
use crate::{
    container,
    edit::Add,
    file,
    fleet::{self, Fleet},
    hash,
    package::{Backup, Db, Origin, Package},
    preview::Preview,
    revert::Etc,
    secrets::{self, Storage},
    sign, systemd, walk,
};
use anyhow::{Context, Result};
use std::{
    collections::HashSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

const ACCOUNT_FILES: [&str; 7] = [
    "/etc/passwd",
    "/etc/group",
    "/etc/shadow",
    "/etc/gshadow",
    "/etc/subuid",
    "/etc/subgid",
    "/etc/shells",
];

pub fn scan(fleet: &Fleet, db: &Db) -> Result<Vec<Item>> {
    let declared = &fleet.etc;
    let declared_paths: HashSet<PathBuf> = declared
        .iter()
        .map(|rel| Path::new(file::ETC).join(rel))
        .collect();

    let mut items = Vec::new();
    for rel in declared {
        let path = Path::new(file::ETC).join(rel);
        let secret = fleet.secret_etc.contains(rel);
        if secret && !fleet.etc_open() {
            items.push(Item::locked(Kind::Etc, path.display().to_string()));
            continue;
        }
        let item =
            |change, detail| Item::new(Kind::Etc, change, path.display().to_string(), detail);
        let item = match compare(fleet, rel, &path)? {
            Compare::Same => None,
            Compare::Differs => Some(
                item(Change::Edited, "edited")
                    .with(Add::CopyEtc {
                        rel: rel.clone(),
                        storage: Storage::Clear,
                    })
                    .reverting(Etc::Write(path.clone()))
                    .showing(fleet_side(fleet, rel, Some(&path)))
                    .guessing(&path),
            ),
            Compare::Missing => Some(
                item(Change::Removed, "deleted")
                    .with(Add::RemoveEtc(rel.clone()))
                    .reverting(Etc::Write(path.clone()))
                    .showing(fleet_side(fleet, rel, None)),
            ),
        };
        items.extend(item.map(|item| {
            if secret {
                item.secret(Secret::Fleet)
            } else {
                item
            }
        }));
    }

    let mut changed = Vec::new();
    for package in &db.packages {
        for backup in &package.backups {
            let path = &backup.path;
            if let Ok(rel) = path.strip_prefix(file::ETC)
                && !declared_paths.contains(path)
                && !ACCOUNT_FILES.contains(&path.to_string_lossy().as_ref())
            {
                changed.extend(edited(package, backup, rel)?);
            }
        }
    }
    changed.sort_by(|a, b| a.subject.cmp(&b.subject));
    items.extend(changed);

    let (owned_dirs, owned_files): (Vec<&str>, Vec<&str>) = db
        .packages
        .iter()
        .flat_map(|p| &p.files)
        .map(String::as_str)
        .filter(|path| path.starts_with("/etc/"))
        .partition(|path| path.ends_with('/'));
    let mut known: HashSet<PathBuf> = owned_files.into_iter().map(PathBuf::from).collect();
    known.extend(declared_paths);
    known.extend(ACCOUNT_FILES.iter().map(PathBuf::from));
    known.insert(PathBuf::from(fleet::MACHINE));
    known.insert(PathBuf::from(secrets::ROOT_KEY));
    known.insert(PathBuf::from(sign::KEY));
    known.insert(PathBuf::from(container::account::SYSCTL));
    known.extend(crate::probe::unit::known());
    known.extend(services::global_defaults(db.distro)?);
    let known_dirs = owned_dirs
        .into_iter()
        .map(|dir| PathBuf::from(dir.trim_end_matches('/')))
        .collect();
    let walk = walk::unknown(
        Path::new(file::ETC),
        &known,
        &known_dirs,
        &systemd::is_enablement,
        &|path: &Path| fleet.ignore.matches(Kind::Etc, &path.display().to_string()),
    )?;
    for path in &walk.unknown {
        let rel = path.strip_prefix(file::ETC)?.to_path_buf();
        items.push(
            Item::new(Kind::Etc, Change::Added, path.display().to_string(), "new")
                .with(Add::CopyEtc {
                    rel,
                    storage: Storage::Clear,
                })
                .reverting(Etc::Delete(path.clone()))
                .showing(Preview::Contents(path.clone()))
                .guessing(path),
        );
    }
    items.extend(walk.unreadable.into_iter().map(|place| {
        Item::new(
            Kind::Etc,
            Change::Unreadable,
            place.path.display().to_string(),
            "unreadable",
        )
        .noting(place.why)
    }));
    Ok(items)
}

fn fleet_side(fleet: &Fleet, rel: &Path, now: Option<&Path>) -> Preview {
    let source = fleet.etc_source(rel);
    match (fleet.secret_etc.contains(rel), now) {
        (true, now) => Preview::Decrypted {
            source,
            keys: fleet.root_keys().clone(),
            now: now.map(Path::to_path_buf),
        },
        (false, Some(now)) => Preview::Diff {
            old: source,
            new: now.to_path_buf(),
        },
        (false, None) => Preview::Contents(source),
    }
}

enum Compare {
    Same,
    Differs,
    Missing,
}

fn compare(fleet: &Fleet, rel: &Path, path: &Path) -> Result<Compare> {
    let source = fleet.etc_source(rel);
    let same = if fs::symlink_metadata(&source)?.is_symlink() {
        let wanted = fs::read_link(&source)?;
        fs::read_link(path).map(|target| target == wanted)
    } else {
        let wanted = fleet.etc_contents(rel)?;
        fs::read(path).map(|bytes| bytes == wanted)
    };
    Ok(match same {
        Ok(true) => Compare::Same,
        Ok(false) => Compare::Differs,
        Err(e) if e.kind() == ErrorKind::NotFound => Compare::Missing,
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    })
}

fn edited(package: &Package, backup: &Backup, rel: &Path) -> Result<Option<Item>> {
    let path = &backup.path;
    let item = |change, detail| Item::new(Kind::Etc, change, path.display().to_string(), detail);
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    if hash::md5(&bytes) == backup.md5 {
        return Ok(None);
    }
    let item = item(Change::Edited, "edited").with(Add::CopyEtc {
        rel: rel.to_path_buf(),
        storage: Storage::Clear,
    });
    let item = match backup.origin {
        Origin::Archive if package.archive.cached() => item
            .noting(format!("owned by {}", package.name))
            .reverting(Etc::Restore {
                path: path.clone(),
                archive: package.archive.clone(),
            })
            .showing(Preview::Original {
                path: path.clone(),
                archive: package.archive.clone(),
            }),
        Origin::Archive => item
            .noting(format!(
                "owned by {}, whose archive isn't in the package cache to restore it from",
                package.name
            ))
            .showing(Preview::Contents(path.clone())),
        Origin::Script => item
            .noting(format!("written by {}'s install script", package.name))
            .showing(Preview::Contents(path.clone())),
    };
    Ok(Some(item.guessing(path)))
}
