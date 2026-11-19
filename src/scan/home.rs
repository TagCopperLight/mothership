use crate::item::{Change, Item};
use crate::kind::Kind;
use crate::{
    edit::Add,
    fleet::{Manager, Member},
    preview::Preview,
    secrets::{self, Storage},
    systemd, walk,
};
use anyhow::Result;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

pub fn scan(member: &Member, managed: &[PathBuf]) -> Result<Vec<Item>> {
    let user = member.user();
    let home = &user.home;
    let tools = &member.config.tools;
    let mut known: HashSet<PathBuf> = managed.iter().cloned().collect();
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
    let uv = home.join(".local/share/uv");
    let uv_links = user.with_rights(|| {
        uv.exists()
            .then(|| links_into(&home.join(".local/bin"), &uv.join("tools")))
            .transpose()
    })?;
    if let Some(links) = uv_links {
        known.extend(links);
        known.insert(uv);
    }

    let skip = |path: &Path| {
        let visible = path.parent() == Some(home.as_path())
            && !path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with('.'));
        visible || systemd::is_enablement(path)
    };
    let ignored = |path: &Path| member.ignore.matches(Kind::Home, &user.tilde(path));
    let walk =
        user.with_rights(|| walk::unknown(home, &known, &HashSet::new(), &skip, &ignored))?;
    let mut items: Vec<Item> = walk
        .unknown
        .iter()
        .map(|path| {
            Item::new(Kind::Home, Change::Added, user.tilde(path), "new")
                .with(Add::Track {
                    path: path.clone(),
                    storage: Storage::Clear,
                })
                .showing(Preview::Home {
                    path: path.clone(),
                    user: user.clone(),
                })
                .guessing(path)
        })
        .collect();
    items.extend(walk.unreadable.into_iter().map(|place| {
        Item::new(
            Kind::Home,
            Change::Unreadable,
            user.tilde(&place.path),
            "unreadable",
        )
        .noting(place.why)
    }));
    Ok(items)
}

fn links_into(bin: &Path, dir: &Path) -> Result<Vec<PathBuf>> {
    let mut links = Vec::new();
    for path in walk::children(bin)? {
        if fs::symlink_metadata(&path)?.is_symlink() && fs::read_link(&path)?.starts_with(dir) {
            links.push(path);
        }
    }
    Ok(links)
}

#[cfg(test)]
#[path = "../../tests/scan_home.rs"]
mod tests;
