use super::{Item, Kind, looks_secret, services};
use crate::{cmd, fleet::Fleet, walk};
use anyhow::{Context, Result};
use std::{
    collections::{BTreeSet, HashSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Command,
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

pub fn scan(fleet: &Fleet) -> Result<Vec<Item>> {
    let fleet_etc = fleet.type_dir.join("etc");
    let declared: Vec<PathBuf> = walk::files(&fleet_etc)?;
    let declared_paths: HashSet<PathBuf> = declared
        .iter()
        .map(|rel| Path::new("/etc").join(rel))
        .collect();

    let mut items = Vec::new();
    for rel in &declared {
        let path = Path::new("/etc").join(rel);
        if let Some(detail) = compare(&fleet_etc.join(rel), &path)? {
            items.push(Item::new(Kind::Etc, path.display().to_string(), detail));
        }
    }

    for path in modified_backups()? {
        if !declared_paths.contains(Path::new(&path)) && !ACCOUNT_FILES.contains(&path.as_str()) {
            items.push(Item::new(
                Kind::Etc,
                path,
                "changed from the package's version",
            ));
        }
    }

    let owned = cmd::stdout("pacman", &["-Qlq"])?;
    let (owned_dirs, owned_files): (Vec<&str>, Vec<&str>) = owned
        .lines()
        .filter(|line| line.starts_with("/etc/"))
        .partition(|line| line.ends_with('/'));
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
            Item::new(Kind::Etc, path.display().to_string(), "from no package"),
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

fn compare(source: &Path, path: &Path) -> Result<Option<&'static str>> {
    let same = if fs::symlink_metadata(source)?.is_symlink() {
        fs::read_link(path).map(|target| target == fs::read_link(source).unwrap_or_default())
    } else {
        fs::read(path).map(|bytes| bytes == fs::read(source).unwrap_or_default())
    };
    Ok(match same {
        Ok(true) => None,
        Ok(false) => Some("differs from the fleet"),
        Err(e) if e.kind() == ErrorKind::NotFound => Some("missing, the fleet has it"),
        Err(e) if e.kind() == ErrorKind::PermissionDenied => Some("unreadable without root"),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    })
}

/// Package config files edited since install, as `pacman -Qkk` reports them.
fn modified_backups() -> Result<BTreeSet<String>> {
    // pacman exits non-zero whenever it finds a difference, which is what we're after.
    let out = Command::new("pacman")
        .arg("-Qkk")
        .output()
        .context("running pacman -Qkk")?;
    Ok(String::from_utf8(out.stdout)?
        .lines()
        .filter_map(|line| line.strip_prefix("backup file: "))
        .filter_map(|rest| rest.split_once(": ").map(|(_, rest)| rest))
        .filter_map(|rest| rest.split(" (").next())
        .filter(|path| path.starts_with("/etc/"))
        .map(str::to_owned)
        .collect())
}
