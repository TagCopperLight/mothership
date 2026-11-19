use super::{Change, Item, Kind};
use crate::{
    edit::Add,
    fleet::{Fleet, Manager, Member},
    preview::{Preview, Seen},
    secrets, systemd, walk,
};
use anyhow::Result;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

pub fn scan(fleet: &Fleet, member: &Member, managed: &[PathBuf]) -> Result<Vec<Item>> {
    let user = member.user();
    let home = &user.home;
    let tools = &member.config.tools;
    let mut known: HashSet<PathBuf> = managed.iter().cloned().collect();
    known.insert(fleet.dir.clone());
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
        visible || systemd::is_enablement(path)
    };
    let ignored = |path: &Path| member.ignore.matches_subject(Kind::Home, &user.tilde(path));
    let walk =
        user.with_rights(|| walk::unknown(home, &known, &HashSet::new(), &skip, &ignored))?;
    let mut items: Vec<Item> = walk
        .unknown
        .iter()
        .map(|path| {
            Item::new(Kind::Home, Change::Added, user.tilde(path), "new")
                .with(Add::Track {
                    path: path.clone(),
                    encrypt: false,
                })
                .showing(Preview::Home {
                    path: path.clone(),
                    user: user.clone(),
                    seen: Seen::default(),
                })
                .guessing(path)
        })
        .collect();
    items.extend(walk.unreadable.iter().map(|path| {
        Item::new(
            Kind::Home,
            Change::Unreadable,
            user.tilde(path),
            "unreadable",
        )
    }));
    Ok(items)
}
