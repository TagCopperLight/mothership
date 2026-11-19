use anyhow::{Context, Result, bail};
use std::{
    collections::HashSet,
    fs,
    io::{ErrorKind, Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

const READ_LIMIT: u64 = 64 << 20;

pub struct Walk {
    pub unknown: Vec<PathBuf>,
    pub unreadable: Vec<PathBuf>,
}

pub fn unknown(
    root: &Path,
    known: &HashSet<PathBuf>,
    known_dirs: &HashSet<PathBuf>,
    skip: &dyn Fn(&Path) -> bool,
    ignored: &dyn Fn(&Path) -> bool,
) -> Result<Walk> {
    let mut open = known_dirs.clone();
    for path in known.iter().chain(known_dirs) {
        for ancestor in path.ancestors().skip(1) {
            if !open.insert(ancestor.to_path_buf()) {
                break;
            }
        }
    }
    let mut walk = Walk {
        unknown: Vec::new(),
        unreadable: Vec::new(),
    };
    let rules = Rules {
        known,
        open: &open,
        skip,
        ignored,
    };
    visit(root, &rules, &mut walk)?;
    walk.unknown.sort();
    Ok(walk)
}

struct Rules<'a> {
    known: &'a HashSet<PathBuf>,
    open: &'a HashSet<PathBuf>,
    skip: &'a dyn Fn(&Path) -> bool,
    ignored: &'a dyn Fn(&Path) -> bool,
}

fn visit(dir: &Path, rules: &Rules, walk: &mut Walk) -> Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::PermissionDenied => {
            walk.unreadable.push(dir.to_path_buf());
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if rules.known.contains(&path) || (rules.skip)(&path) {
            continue;
        }
        if !entry.file_type()?.is_dir() {
            walk.unknown.push(path);
        } else if rules.open.contains(&path) {
            visit(&path, rules, walk)?;
        } else {
            match contents(&path, rules) {
                Contents::Nothing => {}
                Contents::New => walk.unknown.push(path),
                Contents::SomeIgnored => visit(&path, rules, walk)?,
            }
        }
    }
    Ok(())
}

enum Contents {
    Nothing,
    New,
    SomeIgnored,
}

fn contents(dir: &Path, rules: &Rules) -> Contents {
    if (rules.ignored)(dir) {
        return Contents::New;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return Contents::New;
    };
    let mut found = Contents::Nothing;
    for entry in entries {
        let Ok(entry) = entry else {
            return Contents::New;
        };
        let path = entry.path();
        if (rules.skip)(&path) {
            continue;
        }
        if (rules.ignored)(&path) {
            return Contents::SomeIgnored;
        }
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            found = Contents::New;
            continue;
        }
        match contents(&path, rules) {
            Contents::SomeIgnored => return Contents::SomeIgnored,
            Contents::New => found = Contents::New,
            Contents::Nothing => {}
        }
    }
    found
}

pub fn files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                dirs.push(entry.path());
            } else {
                found.push(entry.path().strip_prefix(root)?.to_path_buf());
            }
        }
    }
    found.sort();
    Ok(found)
}

pub fn children(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        entries => entries.with_context(|| format!("reading {}", dir.display()))?,
    };
    let mut paths = entries
        .map(|entry| Ok(entry?.path()))
        .collect::<Result<Vec<_>>>()?;
    paths.sort();
    Ok(paths)
}

pub fn read_if_any(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

#[cfg(test)]
#[path = "../tests/walk.rs"]
mod tests;

pub fn write(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let dir = path.parent().context("no parent folder")?;
    fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(bytes)?;
    let temp = file.as_file();
    match fs::symlink_metadata(path) {
        Ok(old) if old.is_file() => {
            let new = temp.metadata()?;
            if (old.uid(), old.gid()) != (new.uid(), new.gid()) {
                std::os::unix::fs::fchown(temp, Some(old.uid()), Some(old.gid()))
                    .with_context(|| format!("keeping the owner of {}", path.display()))?;
            }
        }
        Err(e) if e.kind() != ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    temp.set_permissions(fs::Permissions::from_mode(mode))?;
    temp.sync_all()?;
    file.persist(path)?;
    Ok(())
}

pub fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path);
    let mut file = match file {
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        file => file.with_context(|| format!("opening {}", path.display()))?,
    };
    let meta = file.metadata()?;
    if !meta.is_file() {
        bail!("{} isn't a regular file", path.display());
    }
    if meta.len() > READ_LIMIT {
        bail!("{} is over {} MB", path.display(), READ_LIMIT >> 20);
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(READ_LIMIT + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading {}", path.display()))?;
    if bytes.len() as u64 > READ_LIMIT {
        bail!("{} is over {} MB", path.display(), READ_LIMIT >> 20);
    }
    Ok(Some(bytes))
}

pub fn under(root: &Path, absolute: &str) -> PathBuf {
    root.join(absolute.trim_start_matches('/'))
}

pub fn read_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> Result<T> {
    match fs::read_to_string(path) {
        Ok(text) => {
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

pub fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    write(path, serde_json::to_string_pretty(value)?.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}
