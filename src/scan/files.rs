use super::{Change, Item, Kind, Secret, join, looks_secret};
use crate::{
    chezmoi,
    cmd::{self, As},
    edit::Add,
    fleet::Member,
    preview::Preview,
    revert::Revert,
};
use anyhow::Result;
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
        // chezmoi fails on a file it can't decrypt.
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

pub fn scan(member: &Member, chezmoi: &Chezmoi) -> Result<Vec<Item>> {
    let user = member.user();
    let Chezmoi {
        managed,
        encrypted,
        templates,
        status,
    } = chezmoi;
    let mut items = Vec::new();
    let mut listed = BTreeSet::new();
    for line in status.lines() {
        let codes = line.as_bytes();
        let (here, target) = (codes[0], codes[1]);
        // R marks a script that would run, not a file.
        if here == b'R' || target == b'R' {
            continue;
        }
        let edited = here != b' ';
        let (change, detail) = chezmoi_status(if edited { here } else { target }, edited);
        let path = user.home.join(&line[3..]);
        let add = if edited && change == Change::Removed {
            Add::Forget(path.clone())
        } else {
            Add::Update {
                path: path.clone(),
                encrypt: false,
            }
        };
        let item = Item::new(Kind::File, change, format!("~/{}", &line[3..]), detail)
            .with(add)
            .reverting(Revert::ApplyFile {
                path: path.clone(),
                edited,
            })
            .showing(Preview::Chezmoi {
                path: path.clone(),
                user: user.clone(),
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

    // A template takes its secrets from encrypted files.
    for path in managed {
        if listed.contains(path) || encrypted.contains(path) || templates.contains(path) {
            continue;
        }
        let add = Add::Update {
            path: path.clone(),
            encrypt: false,
        };
        let item = looks_secret(
            &member.not_secret,
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

/// chezmoi's first column says what changed here since it last wrote the file, the second what
/// its apply would do. A file chezmoi would add is one the machine lacks.
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
