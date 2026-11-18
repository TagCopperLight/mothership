use super::{Item, Kind};
use crate::{cmd, edit::Add, preview::Preview, revert::Revert, user};
use anyhow::Result;

pub fn scan() -> Result<Vec<Item>> {
    let mut items = Vec::new();
    for line in cmd::stdout("chezmoi", &["status"])?.lines() {
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
        items.push(
            Item::new(Kind::File, format!("~/{}", &line[3..]), detail)
                .with(Add::Update(path.clone()))
                .reverting(Revert::ApplyFile {
                    path: path.clone(),
                    edited,
                })
                .showing(Preview::Chezmoi(path)),
        );
    }
    Ok(items)
}
