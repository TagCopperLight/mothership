use crate::sync::item::{Change, Item};
use crate::sync::kind::Kind;
use crate::{
    fleet::edit::Add,
    fleet::{Manager, Member},
    secrets::{self, Storage},
    sync::preview::Preview,
    system::systemd,
    trust::ssh,
    util::walk,
};
use anyhow::Result;
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub fn scan(member: &Member, managed: &[PathBuf]) -> Result<Vec<Item>> {
    let user = member.user();
    let home = &user.home;
    let tools = &member.config.tools;
    let mut known: HashSet<PathBuf> = managed.iter().cloned().collect();
    known.insert(home.join(".config/chezmoi"));
    known.insert(secrets::key_path(user));
    known.insert(ssh::key_path(user));
    known.insert(ssh::public_path(user));
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
    let apps = home.join(".local/opt");
    let bin = home.join(".local/bin");
    let opened = HashSet::from([apps.clone(), bin.clone()]);
    let (walk, downloaded) = user.with_rights(|| {
        let walk = walk::unknown(home, &known, &opened, &skip, &ignored)?;
        let mut downloaded = HashSet::new();
        for path in &walk.unknown {
            if path.parent() == Some(&apps) || (path.parent() == Some(&bin) && compiled(path)?) {
                downloaded.insert(path.clone());
            }
        }
        Ok((walk, downloaded))
    })?;
    let mut items: Vec<Item> = walk
        .unknown
        .iter()
        .map(|path| match downloaded.contains(path) {
            true => Item::new(Kind::Tool, Change::Added, user.tilde(path), "installed")
                .because("nothing in the fleet downloads it: write an installer for it"),
            false => Item::new(Kind::Home, Change::Added, user.tilde(path), "new")
                .with(Add::Track {
                    path: path.clone(),
                    storage: Storage::Clear,
                })
                .showing(Preview::Home {
                    path: path.clone(),
                    user: user.clone(),
                })
                .guessing(path),
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

fn compiled(path: &Path) -> Result<bool> {
    if !fs::symlink_metadata(path)?.is_file() {
        return Ok(false);
    }
    let mut magic = [0; 4];
    let read = fs::File::open(path)?.read(&mut magic)?;
    Ok(read == magic.len() && magic == *b"\x7fELF")
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
#[path = "../../../tests/sync/scan/home.rs"]
mod tests;
