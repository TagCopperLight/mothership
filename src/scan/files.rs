use super::{Change, Item, Kind, Secret, join, looks_secret};
use crate::{
    chezmoi,
    cmd::{self, As},
    edit::Add,
    fleet::{Fleet, Member},
    preview::{Preview, Seen},
    revert::User,
    secrets::Storage,
};
use anyhow::{Context, Result};
use std::{collections::BTreeSet, path::PathBuf, thread};

pub struct Chezmoi {
    pub managed: Vec<PathBuf>,
    encrypted: BTreeSet<PathBuf>,
    templates: BTreeSet<PathBuf>,
    status: String,
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
            let status = s.spawn(|| cmd::stdout(As::User(user), "chezmoi", status_args));
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
    let root = chezmoi::Root::new(fleet, member);
    for line in status.lines() {
        let (here, target, rel) =
            status_line(line).with_context(|| format!("chezmoi status printed {line:?}"))?;
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
                seen: Seen::default(),
            });
        items.push(if encrypted.contains(&path) {
            item.secret(Secret::Fleet)
        } else {
            item.guessing(&path)
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
            .with(add),
            path,
        );
        if item.secret == Secret::Guessed {
            items.push(item);
        }
    }
    Ok(items)
}

fn status_line(line: &str) -> Option<(u8, u8, &str)> {
    match line.as_bytes() {
        [here, target, b' ', _, ..] => Some((*here, *target, line.get(3..)?)),
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
#[path = "../../tests/files.rs"]
mod tests;
