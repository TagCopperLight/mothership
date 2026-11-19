use anyhow::Result;
use std::{
    collections::HashSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

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

#[cfg(test)]
#[path = "../tests/walk.rs"]
mod tests;

pub fn under(root: &Path, absolute: &str) -> PathBuf {
    root.join(absolute.trim_start_matches('/'))
}
