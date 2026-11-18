use super::{Item, Kind, Secret, join, looks_secret};
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
        let detail = if edited {
            "edited here"
        } else {
            "changed in the repo, not applied"
        };
        let path = user.home.join(&line[3..]);
        let item = Item::new(Kind::File, format!("~/{}", &line[3..]), detail)
            .with(Add::Update {
                path: path.clone(),
                encrypt: false,
            })
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
            Item::new(Kind::File, user.tilde(path), "in the fleet in clear").with(add),
            path,
        );
        if item.secret == Secret::Guessed {
            items.push(item);
        }
    }
    Ok(items)
}
