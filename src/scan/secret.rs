use crate::{
    fleet::{Fleet, Member},
    ignore::Paths,
    item::{Change, Item, Secret},
    style, walk,
};
use anyhow::Result;
use std::{
    fs,
    io::ErrorKind,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub fn finish(fleet: &Fleet, mut item: Item) -> Item {
    let Some(path) = item.guess.take() else {
        return item;
    };
    let (found, not_secret) = match item.member.map(|i| &fleet.members[i]) {
        None => (Found::at(&path, Path::new(walk::ETC)), &fleet.not_secret),
        Some(member) => {
            let user = member.user();
            let found = user
                .with_rights(|| Ok(Found::at(&path, &user.home)))
                .unwrap_or_else(Found::Unreadable);
            (found, &member.not_secret)
        }
    };
    if item.change == Change::Added && item.note.is_empty() {
        item.note = found.describe();
    }
    judge_secret(not_secret, item, &found)
}

enum Found {
    Missing,
    Link(PathBuf),
    Folder { files: usize, private: bool },
    File { private: bool },
    Unreadable(anyhow::Error),
}

impl Found {
    fn at(path: &Path, base: &Path) -> Found {
        let found = match fs::symlink_metadata(path) {
            Err(e) if e.kind() == ErrorKind::NotFound => return Found::Missing,
            found => found,
        };
        let found = found.map_err(anyhow::Error::from).and_then(|meta| {
            if meta.is_symlink() {
                return Ok(Found::Link(fs::read_link(path)?));
            }
            let hidden = hidden_below(path, base)?;
            if !meta.is_dir() {
                return Ok(Found::File {
                    private: hidden || private(&meta),
                });
            }
            let files = walk::files_with_metadata(path)?;
            let mut private = hidden || closed(&meta);
            for (rel, meta) in &files {
                private |= self::private(meta) || hidden_below(&path.join(rel), path)?;
            }
            Ok(Found::Folder {
                files: files.len(),
                private,
            })
        });
        found.unwrap_or_else(Found::Unreadable)
    }

    fn describe(&self) -> String {
        match self {
            Found::Link(target) => format!("→ {}", target.display()),
            Found::Folder { files, .. } => style::count(*files, "file"),
            Found::Missing | Found::File { .. } => String::new(),
            Found::Unreadable(e) => format!("can't read it: {e:#}"),
        }
    }

    fn private(&self) -> bool {
        match self {
            Found::Missing | Found::Link(_) => false,
            Found::Folder { private, .. } | Found::File { private } => *private,
            Found::Unreadable(_) => true,
        }
    }
}

fn hidden_below(path: &Path, base: &Path) -> Result<bool> {
    for dir in path.ancestors().skip(1).take_while(|dir| *dir != base) {
        if !dir.starts_with(base) {
            break;
        }
        if closed(&fs::metadata(dir)?) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn closed(dir: &fs::Metadata) -> bool {
    dir.permissions().mode() & 0o001 == 0
}

pub fn looks_secret(member: &Member, item: Item, path: &Path) -> Item {
    let user = member.user();
    let found = user
        .with_rights(|| Ok(Found::at(path, &user.home)))
        .unwrap_or_else(Found::Unreadable);
    judge_secret(&member.not_secret, item, &found)
}

fn judge_secret(not_secret: &Paths, item: Item, found: &Found) -> Item {
    if !not_secret.matches(&item.subject) && found.private() {
        item.secret(Secret::Guessed)
    } else {
        item
    }
}

fn private(meta: &fs::Metadata) -> bool {
    meta.is_file() && meta.permissions().mode() & 0o004 == 0
}

#[cfg(test)]
#[path = "../../tests/secret.rs"]
mod tests;
