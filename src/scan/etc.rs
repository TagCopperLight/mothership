use super::{Change, Item, Kind, Secret, services};
use crate::{
    edit::Add,
    fleet::{self, Fleet},
    package::{self, Backup, Db, Origin, Package},
    preview::Preview,
    revert::Revert,
    secrets, walk,
};
use anyhow::{Context, Result};
use std::{
    collections::HashSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

/// User and group changes rewrite these, and the account check covers them.
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
        .map(|rel| Path::new("/etc").join(rel))
        .collect();

    let mut items = Vec::new();
    for rel in declared {
        let path = Path::new("/etc").join(rel);
        let secret = fleet.secret_etc.contains(rel);
        if secret && !fleet.etc_open() {
            items.push(Item::locked(Kind::Etc, path.display().to_string()));
            continue;
        }
        let source = fleet.etc_source(rel);
        let item =
            |change, detail| Item::new(Kind::Etc, change, path.display().to_string(), detail);
        let item = match compare(fleet, rel, &path)? {
            Compare::Same => None,
            Compare::Differs => Some(
                item(Change::Edited, "edited")
                    .with(Add::CopyEtc {
                        path: path.clone(),
                        encrypt: false,
                    })
                    .reverting(Revert::WriteEtc(path.clone()))
                    .showing(Preview::Diff {
                        old: source,
                        new: path.clone(),
                    })
                    .guessing(&path),
            ),
            Compare::Missing => Some(
                item(Change::Removed, "deleted")
                    .with(Add::RemoveEtc(path.clone()))
                    .reverting(Revert::WriteEtc(path.clone()))
                    .showing(Preview::Contents(source)),
            ),
            Compare::Unreadable => Some(item(Change::Unreadable, "unreadable")),
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
            if path.starts_with("/etc")
                && !declared_paths.contains(path)
                && !ACCOUNT_FILES.contains(&path.to_string_lossy().as_ref())
            {
                changed.extend(edited(package, backup)?);
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
    let known_dirs = owned_dirs
        .into_iter()
        .map(|dir| PathBuf::from(dir.trim_end_matches('/')))
        .collect();
    let walk = walk::unknown(
        Path::new("/etc"),
        &known,
        &known_dirs,
        &services::is_enablement,
        &|path: &Path| {
            fleet
                .ignore
                .matches_subject(Kind::Etc, &path.display().to_string())
        },
    )?;
    items.extend(walk.unknown.iter().map(|path| {
        Item::new(Kind::Etc, Change::Added, path.display().to_string(), "new")
            .with(Add::CopyEtc {
                path: path.clone(),
                encrypt: false,
            })
            .reverting(Revert::DeleteEtc(path.clone()))
            .showing(Preview::Contents(path.clone()))
            .guessing(path)
    }));
    items.extend(walk.unreadable.iter().map(|path| {
        Item::new(
            Kind::Etc,
            Change::Unreadable,
            path.display().to_string(),
            "unreadable",
        )
    }));
    Ok(items)
}

enum Compare {
    Same,
    Differs,
    Missing,
    Unreadable,
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
        Err(e) if e.kind() == ErrorKind::PermissionDenied => Compare::Unreadable,
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    })
}

fn edited(package: &Package, backup: &Backup) -> Result<Option<Item>> {
    let path = &backup.path;
    let item = |change, detail| Item::new(Kind::Etc, change, path.display().to_string(), detail);
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        // The package manager leaves a config file the package no longer ships, or that I deleted.
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) if e.kind() == ErrorKind::PermissionDenied => {
            return Ok(Some(item(Change::Unreadable, "unreadable")));
        }
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    if package::md5(&bytes) == backup.md5 {
        return Ok(None);
    }
    let item = item(Change::Edited, "edited").with(Add::CopyEtc {
        path: path.clone(),
        encrypt: false,
    });
    let item = match backup.origin {
        Origin::Archive => item
            .noting(format!("owned by {}", package.name))
            .reverting(Revert::RestoreEtc {
                path: path.clone(),
                archive: package.archive.clone(),
            })
            .showing(Preview::Original {
                path: path.clone(),
                archive: package.archive.clone(),
            }),
        Origin::Script => item
            .noting(format!("written by {}'s install script", package.name))
            .showing(Preview::Contents(path.clone())),
    };
    Ok(Some(item.guessing(path)))
}
