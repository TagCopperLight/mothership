use super::join;
use super::secret::looks_secret;
use crate::sync::item::{Change, Item, Secret};
use crate::sync::kind::Kind;
use crate::{
    fleet::edit::Add,
    fleet::{Fleet, Member},
    home::chezmoi,
    secrets::Storage,
    sync::preview::Preview,
    sync::revert::User,
    util::cmd::{self, As},
};
use anyhow::{Context, Result};
use std::{
    collections::BTreeSet,
    ffi::OsStr,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    thread,
};

pub struct Chezmoi {
    pub managed: Vec<PathBuf>,
    encrypted: BTreeSet<PathBuf>,
    templates: BTreeSet<PathBuf>,
    status: Vec<u8>,
}

impl Chezmoi {
    pub fn load(member: &Member) -> Result<Self> {
        let user = member.user();
        let status_args: &[&str] = if member.open {
            &["status"]
        } else {
            &["status", "--exclude", "encrypted"]
        };
        thread::scope(|s| {
            let managed = s.spawn(|| chezmoi::managed(user, "files,symlinks"));
            let encrypted = s.spawn(|| chezmoi::managed(user, "encrypted"));
            let templates = s.spawn(|| chezmoi::managed(user, "templates"));
            let status = s.spawn(|| cmd::bytes(As::User(user), "chezmoi", status_args));
            Ok(Chezmoi {
                managed: join(managed)?,
                encrypted: join(encrypted)?.into_iter().collect(),
                templates: join(templates)?.into_iter().collect(),
                status: join(status)?,
            })
        })
    }
}

pub fn scan(fleet: &Fleet, member: &Member, chezmoi: &Chezmoi) -> Result<Vec<Item>> {
    let user = member.user();
    let Chezmoi {
        managed,
        encrypted,
        templates,
        status,
    } = chezmoi;
    let mut items = Vec::new();
    let mut listed = BTreeSet::new();
    let root = member.chezmoi_root(fleet);
    for line in status
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let (here, target, rel) = status_line(line).with_context(|| {
            format!("chezmoi status printed {:?}", String::from_utf8_lossy(line))
        })?;
        if here == b'R' || target == b'R' {
            continue;
        }
        let edited = here != b' ';
        let (change, detail) = chezmoi_status(if edited { here } else { target }, edited);
        let path = chezmoi::in_home(user, &user.home.join(rel))?;
        let add = if edited && change == Change::Removed {
            Add::Forget(path.clone())
        } else {
            Add::Update {
                path: path.clone(),
                storage: Storage::Clear,
            }
        };
        let item = Item::new(Kind::File, change, user.tilde(&path), detail)
            .with(add)
            .reverting(User::ApplyFile {
                path: path.clone(),
                edited,
            })
            .showing(Preview::Chezmoi {
                path: path.clone(),
                root: root.clone(),
            });
        items.push(if encrypted.contains(&path) {
            item.secret(Secret::Fleet)
        } else {
            item.guessing_tracked(&path)
        });
        listed.insert(path);
    }
    if !member.open {
        items.extend(
            encrypted
                .iter()
                .map(|path| Item::locked(Kind::File, user.tilde(path))),
        );
    }

    for path in managed {
        if listed.contains(path) || encrypted.contains(path) || templates.contains(path) {
            continue;
        }
        let add = Add::Update {
            path: path.clone(),
            storage: Storage::Clear,
        };
        let item = looks_secret(
            member,
            Item::new(
                Kind::File,
                Change::InClear,
                user.tilde(path),
                "secret in clear",
            )
            .noting("git history keeps it: change it once encrypted")
            .with(add)
            .showing(Preview::Home {
                path: path.clone(),
                user: user.clone(),
            }),
            path,
        );
        if item.secret == Secret::Guessed {
            items.push(item);
        }
    }
    Ok(items)
}

fn status_line(line: &[u8]) -> Option<(u8, u8, &Path)> {
    match line {
        [here, target, b' ', rest @ ..] if !rest.is_empty() => {
            Some((*here, *target, Path::new(OsStr::from_bytes(rest))))
        }
        _ => None,
    }
}

fn chezmoi_status(code: u8, edited: bool) -> (Change, &'static str) {
    match (code, edited) {
        (b'A', true) => (Change::Added, "created"),
        (b'D', true) => (Change::Removed, "deleted"),
        (_, true) => (Change::Edited, "edited"),
        (b'A', false) => (Change::Removed, "missing"),
        (b'D', false) => (Change::Added, "dropped by the fleet"),
        (_, false) => (Change::Edited, "changed in the fleet"),
    }
}

#[cfg(test)]
#[path = "../../../tests/sync/scan/files.rs"]
mod tests;
