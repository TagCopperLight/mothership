use super::{Item, Kind, guess_secret, looks_secret};
use crate::{
    cmd,
    edit::Add,
    fleet::{Fleet, tilde},
    preview::Preview,
    revert::Revert,
    user,
};
use anyhow::Result;
use std::{collections::BTreeSet, path::PathBuf};

pub fn scan(fleet: &Fleet, managed: &[PathBuf]) -> Result<Vec<Item>> {
    let open = fleet.secrets().open;
    let encrypted = managed_with("encrypted")?;
    let templates = managed_with("templates")?;
    // chezmoi fails on a file it can't decrypt.
    let status = if open {
        cmd::stdout("chezmoi", &["status"])?
    } else {
        cmd::stdout("chezmoi", &["status", "--exclude", "encrypted"])?
    };
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
        let path = user::home().join(&line[3..]);
        let item = Item::new(Kind::File, format!("~/{}", &line[3..]), detail)
            .with(Add::Update {
                path: path.clone(),
                encrypt: false,
            })
            .reverting(Revert::ApplyFile {
                path: path.clone(),
                edited,
            })
            .showing(Preview::Chezmoi(path.clone()));
        items.push(if encrypted.contains(&path) {
            item.secret()
        } else {
            looks_secret(item, &path)
        });
        listed.insert(path);
    }
    if !open {
        items.extend(
            encrypted
                .iter()
                .map(|path| Item::locked(Kind::File, tilde(path))),
        );
    }

    // A template takes its secrets from encrypted files.
    for path in managed {
        if !listed.contains(path)
            && !encrypted.contains(path)
            && !templates.contains(path)
            && guess_secret(path)
        {
            let add = Add::Update {
                path: path.clone(),
                encrypt: false,
            };
            items.push(
                Item::new(Kind::File, tilde(path), "in the fleet in clear")
                    .with(add)
                    .secret(),
            );
        }
    }
    Ok(items)
}

fn managed_with(entries: &str) -> Result<BTreeSet<PathBuf>> {
    let args = ["managed", "--include", entries, "--path-style", "absolute"];
    Ok(cmd::stdout("chezmoi", &args)?
        .lines()
        .map(PathBuf::from)
        .collect())
}
