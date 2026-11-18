use super::{Item, Kind, services};
use crate::{
    edit::Add,
    fleet::{Fleet, Manager, Member},
    preview::Preview,
    secrets, walk,
};
use anyhow::Result;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

/// Only dotfiles at the top of ~ count: the visible folders hold data, not config.
pub fn scan(fleet: &Fleet, member: &Member, managed: &[PathBuf]) -> Result<Vec<Item>> {
    let user = member.user();
    let home = &user.home;
    let tools = &member.config.tools;
    let mut known: HashSet<PathBuf> = managed.iter().cloned().collect();
    // The checkout, when it lives in a home.
    known.insert(fleet.dir.clone());
    // chezmoi's config and the user's key, and the database the dconf check reads.
    known.insert(home.join(".config/chezmoi"));
    known.insert(secrets::key_path(user));
    known.insert(home.join(".config/dconf"));
    known.extend(
        member
            .config
            .installers
            .iter()
            .map(|i| user.expand(&i.creates)),
    );
    if !tools.of(Manager::Cargo).is_empty() {
        known.insert(home.join(".cargo"));
    }
    if !tools.of(Manager::Uv).is_empty() {
        known.insert(home.join(".local/share/uv"));
        known.extend(
            tools
                .of(Manager::Uv)
                .iter()
                .map(|tool| home.join(".local/bin").join(tool.name())),
        );
    }

    let skip = |path: &Path| {
        let visible = path.parent() == Some(home.as_path())
            && !path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with('.'));
        visible || services::is_enablement(path)
    };
    let walk = walk::unknown(home, &known, &HashSet::new(), &skip)?;
    let mut items: Vec<Item> = walk
        .unknown
        .iter()
        .map(|path| {
            Item::new(Kind::Home, user.tilde(path), "not in the fleet")
                .with(Add::Track {
                    path: path.clone(),
                    encrypt: false,
                })
                .showing(Preview::Contents(path.clone()))
                .guessing(path)
        })
        .collect();
    items.extend(
        walk.unreadable
            .iter()
            .map(|path| Item::new(Kind::Home, user.tilde(path), "unreadable")),
    );
    Ok(items)
}
