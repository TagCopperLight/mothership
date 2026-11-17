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

/// An unknown folder is reported whole, so one new app gives one line. Folders in `known_dirs`
/// and folders that hold a known path are opened.
pub fn unknown(
    root: &Path,
    known: &HashSet<PathBuf>,
    known_dirs: &HashSet<PathBuf>,
    skip: &dyn Fn(&Path) -> bool,
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
    visit(root, known, &open, skip, &mut walk)?;
    walk.unknown.sort();
    Ok(walk)
}

fn visit(
    dir: &Path,
    known: &HashSet<PathBuf>,
    open: &HashSet<PathBuf>,
    skip: &dyn Fn(&Path) -> bool,
    walk: &mut Walk,
) -> Result<()> {
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
        if known.contains(&path) || skip(&path) {
            continue;
        }
        let is_dir = entry.file_type()?.is_dir();
        if open.contains(&path) && is_dir {
            visit(&path, known, open, skip, walk)?;
        } else if !is_dir || holds_config(&path, skip) {
            walk.unknown.push(path);
        }
    }
    Ok(())
}

/// A folder holding only skipped paths and empty folders holds no config. One we can't read
/// isn't known to be empty.
fn holds_config(dir: &Path, skip: &dyn Fn(&Path) -> bool) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return true;
    };
    entries.into_iter().any(|entry| {
        let Ok(entry) = entry else { return true };
        let path = entry.path();
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        !skip(&path) && (!is_dir || holds_config(&path, skip))
    })
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
