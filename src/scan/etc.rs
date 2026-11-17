use super::{Item, Kind, looks_secret, services};
use crate::{
    edit::Add,
    fleet::Fleet,
    pacman::{Db, Package},
    preview::Preview,
    walk,
};
use anyhow::{Context, Result};
use md5::{Digest, Md5};
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
    let fleet_etc = fleet.type_dir.join("etc");
    let declared: Vec<PathBuf> = walk::files(&fleet_etc)?;
    let declared_paths: HashSet<PathBuf> = declared
        .iter()
        .map(|rel| Path::new("/etc").join(rel))
        .collect();

    let mut items = Vec::new();
    for rel in &declared {
        let path = Path::new("/etc").join(rel);
        let source = fleet_etc.join(rel);
        let item = |detail| Item::new(Kind::Etc, path.display().to_string(), detail);
        items.extend(match compare(&source, &path)? {
            Compare::Same => None,
            Compare::Differs => Some(
                item("differs from the fleet")
                    .with(Add::CopyEtc(path.clone()))
                    .showing(Preview::Diff {
                        old: source,
                        new: path.clone(),
                    }),
            ),
            Compare::Missing => Some(
                item("missing, the fleet has it")
                    .with(Add::RemoveEtc(path.clone()))
                    .showing(Preview::Contents(source)),
            ),
            Compare::Unreadable => Some(item("unreadable without root")),
        });
    }

    let mut changed = Vec::new();
    for package in &db.packages {
        for (path, md5) in &package.backups {
            if path.starts_with("/etc")
                && !declared_paths.contains(path)
                && !ACCOUNT_FILES.contains(&path.to_string_lossy().as_ref())
            {
                changed.extend(edited(package, path, md5)?);
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
            Item::new(Kind::Etc, path.display().to_string(), "from no package")
                .with(Add::CopyEtc(path.clone()))
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

fn compare(source: &Path, path: &Path) -> Result<Compare> {
    let same = if fs::symlink_metadata(source)?.is_symlink() {
        fs::read_link(path).map(|target| target == fs::read_link(source).unwrap_or_default())
    } else {
        fs::read(path).map(|bytes| bytes == fs::read(source).unwrap_or_default())
    };
    Ok(match same {
        Ok(true) => Compare::Same,
        Ok(false) => Compare::Differs,
        Err(e) if e.kind() == ErrorKind::NotFound => Compare::Missing,
        Err(e) if e.kind() == ErrorKind::PermissionDenied => Compare::Unreadable,
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    })
}

fn edited(package: &Package, path: &Path, md5: &str) -> Result<Option<Item>> {
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
    let actual: String = Md5::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((actual != md5).then(|| {
        item("changed from the package's version")
            .with(Add::CopyEtc(path.to_path_buf()))
            .showing(Preview::Original {
                path: path.to_path_buf(),
                archive: package.archive(),
            })
    }))
}
