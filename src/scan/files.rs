use super::{Item, Kind};
use crate::{cmd, edit::Add, fleet};
use anyhow::Result;

/// Files chezmoi manages that differ from the repo.
pub fn scan() -> Result<Vec<Item>> {
    let mut items = Vec::new();
    for line in cmd::stdout("chezmoi", &["status"])?.lines() {
        let codes = line.as_bytes();
        let (here, target) = (codes[0], codes[1]);
        // R marks a script that would run, not a file.
        if here == b'R' || target == b'R' {
            continue;
        }
        let detail = if here != b' ' {
            "edited here"
        } else {
            "changed in the repo, not applied"
        };
        let path = &line[3..];
        items.push(
            Item::new(Kind::File, format!("~/{path}"), detail)
                .with(Add::Update(fleet::home().join(path))),
        );
    }
    Ok(items)
}
