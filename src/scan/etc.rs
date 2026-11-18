use super::{Item, Kind, Secret, looks_secret, services};
use crate::{
    edit::Add,
    fleet::{self, Fleet},
    pacman::{self, Db, Package},
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
        let item = |detail| Item::new(Kind::Etc, path.display().to_string(), detail);
        let item = match compare(fleet, rel, &path)? {
            Compare::Same => None,
            Compare::Differs => Some(
                item("differs from the fleet")
                    .with(Add::CopyEtc {
                        path: path.clone(),
                        encrypt: false,
                    })
                    .reverting(Revert::WriteEtc(path.clone()))
                    .showing(Preview::Diff {
                        old: source,
                        new: path.clone(),
                    }),
            ),
            Compare::Missing => Some(
                item("missing, the fleet has it")
                    .with(Add::RemoveEtc(path.clone()))
                    .reverting(Revert::WriteEtc(path.clone()))
                    .showing(Preview::Contents(source)),
            ),
            Compare::Unreadable => Some(item("unreadable without root")),
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
        for (path, md5) in &package.backups {
            if path.starts_with("/etc")
                && !declared_paths.contains(path)
                && !ACCOUNT_FILES.contains(&path.to_string_lossy().as_ref())
            {
                changed.extend(edited(fleet, package, path, md5)?);
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
    // The machine's own type and key.
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
    )?;
    items.extend(walk.unknown.iter().map(|path| {
        looks_secret(
            &fleet.not_secret,
            Item::new(Kind::Etc, path.display().to_string(), "from no package")
                .with(Add::CopyEtc {
                    path: path.clone(),
                    encrypt: false,
                })
                .reverting(Revert::DeleteEtc(path.clone()))
                .showing(Preview::Contents(path.clone())),
            path,
        )
    }));
    items.extend(walk.unreadable.iter().map(|path| {
        Item::new(
            Kind::Etc,
            path.display().to_string(),
            "unreadable without root",
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
        fs::read_link(path).map(|target| target == fs::read_link(&source).unwrap_or_default())
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

fn edited(fleet: &Fleet, package: &Package, path: &Path, md5: &str) -> Result<Option<Item>> {
    let item = |detail| Item::new(Kind::Etc, path.display().to_string(), detail);
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        // pacman leaves a config file the package no longer ships, or that I deleted.
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) if e.kind() == ErrorKind::PermissionDenied => {
            return Ok(Some(item("unreadable without root")));
        }
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    Ok((pacman::md5(&bytes) != md5).then(|| {
        let item = item("changed from the package's version")
            .with(Add::CopyEtc {
                path: path.to_path_buf(),
                encrypt: false,
            })
            .reverting(Revert::RestoreEtc {
                path: path.to_path_buf(),
                archive: package.archive(),
            })
            .showing(Preview::Original {
                path: path.to_path_buf(),
                archive: package.archive(),
            });
        looks_secret(&fleet.not_secret, item, path)
    }))
}
