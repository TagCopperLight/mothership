use super::{Item, Kind, looks_secret, services};
use crate::{
    edit::Add,
    fleet::{self, Fleet},
    preview::Preview,
    walk,
};
use anyhow::Result;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

/// Only dotfiles at the top of ~ count: the visible folders hold data, not config.
pub fn scan(fleet: &Fleet, managed: &[PathBuf]) -> Result<Vec<Item>> {
    let home = fleet::home();
    let tools = &fleet.system.tools;
    let mut known: HashSet<PathBuf> = managed.iter().cloned().collect();
    known.insert(fleet.dir.clone());
    // The machine's own type, and the database the dconf check reads.
    known.insert(home.join(".config/chezmoi"));
    known.insert(home.join(".config/dconf"));
    known.extend(
        fleet
            .system
            .installers
            .iter()
            .map(|i| fleet::expand(&i.creates)),
    );
    if !tools.cargo.is_empty() {
        known.insert(home.join(".cargo"));
    }
    if !tools.uv.is_empty() {
        known.insert(home.join(".local/share/uv"));
        known.extend(
            tools
                .uv
                .iter()
                .map(|tool| home.join(".local/bin").join(tool.name())),
        );
    }

    let skip = |path: &Path| {
        let visible = path.parent() == Some(&home)
            && !path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with('.'));
        visible || services::is_enablement(path)
    };
    let walk = walk::unknown(&home, &known, &HashSet::new(), &skip)?;
    let mut items: Vec<Item> = walk
        .unknown
        .iter()
        .map(|path| {
            looks_secret(
                Item::new(Kind::Home, fleet::tilde(path), "not in the fleet")
                    .with(Add::Track(path.clone()))
                    .showing(Preview::Contents(path.clone())),
                path,
            )
        })
        .collect();
    items.extend(
        walk.unreadable
            .iter()
            .map(|path| Item::new(Kind::Home, fleet::tilde(path), "unreadable")),
    );
    Ok(items)
}
