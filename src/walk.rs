use anyhow::{Context, Result};
use std::{
    collections::HashSet,
    fs,
    io::{self, ErrorKind},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

pub struct Walk {
    pub unknown: Vec<PathBuf>,
    pub unreadable: Vec<Unreadable>,
}

pub struct Unreadable {
    pub path: PathBuf,
    pub why: String,
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
        device: fs::metadata(root)
            .with_context(|| format!("reading {}", root.display()))?
            .dev(),
    };
    visit(root, &rules, &mut walk);
    walk.unknown.sort();
    walk.unreadable.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(walk)
}

struct Rules<'a> {
    known: &'a HashSet<PathBuf>,
    open: &'a HashSet<PathBuf>,
    skip: &'a dyn Fn(&Path) -> bool,
    ignored: &'a dyn Fn(&Path) -> bool,
    device: u64,
}

enum Entry {
    File,
    Dir,
    Elsewhere,
    Unreadable(io::Error),
}

fn listing(dir: &Path, rules: &Rules) -> io::Result<Vec<(PathBuf, Entry)>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if rules.known.contains(&path) || (rules.skip)(&path) {
            continue;
        }
        let kind = match entry.metadata() {
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => Entry::Unreadable(e),
            Ok(meta) if meta.is_dir() && meta.dev() != rules.device => Entry::Elsewhere,
            Ok(meta) if meta.is_dir() => Entry::Dir,
            Ok(meta) if meta.is_file() || meta.is_symlink() => Entry::File,
            Ok(_) => Entry::Elsewhere,
        };
        found.push((path, kind));
    }
    Ok(found)
}

fn visit(dir: &Path, rules: &Rules, walk: &mut Walk) {
    let entries = match listing(dir, rules) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return,
        Err(e) => return walk.unreadable(dir.to_path_buf(), e),
    };
    for (path, entry) in entries {
        match entry {
            Entry::Elsewhere => {}
            Entry::Unreadable(e) => walk.unreadable(path, e),
            Entry::File => walk.unknown.push(path),
            Entry::Dir if rules.open.contains(&path) => visit(&path, rules, walk),
            Entry::Dir => match contents(&path, rules) {
                Contents::Nothing => {}
                Contents::New => walk.unknown.push(path),
                Contents::Mixed => visit(&path, rules, walk),
            },
        }
    }
}

impl Walk {
    fn unreadable(&mut self, path: PathBuf, error: io::Error) {
        self.unreadable.push(Unreadable {
            path,
            why: error.to_string(),
        });
    }
}

enum Contents {
    Nothing,
    New,
    Mixed,
}

fn contents(dir: &Path, rules: &Rules) -> Contents {
    if (rules.ignored)(dir) {
        return Contents::New;
    }
    let Ok(entries) = listing(dir, rules) else {
        return Contents::Mixed;
    };
    let mut found = Contents::Nothing;
    for (path, entry) in entries {
        match entry {
            Entry::Elsewhere => {}
            Entry::Unreadable(_) => return Contents::Mixed,
            _ if (rules.ignored)(&path) => return Contents::Mixed,
            Entry::File => found = Contents::New,
            Entry::Dir => match contents(&path, rules) {
                Contents::Mixed => return Contents::Mixed,
                Contents::New => found = Contents::New,
                Contents::Nothing => {}
            },
        }
    }
    found
}

pub fn files(root: &Path) -> Result<Vec<PathBuf>> {
    Ok(files_with_metadata(root)?
        .into_iter()
        .map(|(path, _)| path)
        .collect())
}

pub fn files_with_metadata(root: &Path) -> Result<Vec<(PathBuf, fs::Metadata)>> {
    let mut found = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry?;
            let meta = entry.metadata()?;
            if meta.is_dir() {
                dirs.push(entry.path());
            } else if meta.is_file() || meta.is_symlink() {
                found.push((entry.path().strip_prefix(root)?.to_path_buf(), meta));
            }
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(found)
}

pub fn private(meta: &fs::Metadata) -> bool {
    meta.is_file() && meta.permissions().mode() & 0o004 == 0
}

pub fn children(dir: &Path) -> Result<Vec<PathBuf>> {
    let Some(entries) = crate::file::if_found(fs::read_dir(dir))
        .with_context(|| format!("reading {}", dir.display()))?
    else {
        return Ok(Vec::new());
    };
    let mut paths = entries
        .map(|entry| Ok(entry?.path()))
        .collect::<Result<Vec<_>>>()?;
    paths.sort();
    Ok(paths)
}

#[cfg(test)]
#[path = "../tests/walk.rs"]
mod tests;
