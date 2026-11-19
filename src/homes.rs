use crate::walk;
use anyhow::{Context, Result, bail};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

const HOMES: &str = "homes";
const SHARED: &str = "shared";

pub fn source(fleet_dir: &Path, user: &str) -> PathBuf {
    fleet_dir.join(HOMES).join(user)
}

pub fn check(fleet_dir: &Path) -> Result<()> {
    for link in links(fleet_dir)? {
        target(fleet_dir, &link)?;
    }
    Ok(())
}

pub fn users(fleet_dir: &Path, source: &Path) -> Result<Vec<String>> {
    let is_link = fs::symlink_metadata(source).is_ok_and(|meta| meta.is_symlink());
    if !is_link {
        return Ok(vec![owner(fleet_dir, source)?]);
    }
    let shared = target(fleet_dir, source)?;
    let mut users = Vec::new();
    for link in links(fleet_dir)? {
        if target(fleet_dir, &link)? == shared {
            users.push(owner(fleet_dir, &link)?);
        }
    }
    users.sort();
    users.dedup();
    Ok(users)
}

pub fn target(fleet_dir: &Path, link: &Path) -> Result<PathBuf> {
    let shown = link.strip_prefix(fleet_dir).unwrap_or(link).display();
    let shared = fleet_dir.join(SHARED).canonicalize().with_context(|| {
        format!("{shown} is a link, and the fleet has no {SHARED}/ folder for it to lead to")
    })?;
    let file = match link.canonicalize() {
        Err(e) if e.kind() == ErrorKind::NotFound => bail!("{shown} leads nowhere"),
        file => file.with_context(|| format!("following {shown}"))?,
    };
    if !file.starts_with(&shared) {
        bail!("{shown} leads out of {SHARED}/");
    }
    if file.is_dir() {
        bail!("{shown} leads to a folder, which chezmoi doesn't follow: link each file in it");
    }
    Ok(file)
}

fn links(fleet_dir: &Path) -> Result<Vec<PathBuf>> {
    let homes = fleet_dir.join(HOMES);
    let mut links = Vec::new();
    for rel in walk::files(&homes)? {
        let path = homes.join(rel);
        if fs::symlink_metadata(&path)?.is_symlink() {
            links.push(path);
        }
    }
    Ok(links)
}

fn owner(fleet_dir: &Path, source: &Path) -> Result<String> {
    source
        .strip_prefix(fleet_dir.join(HOMES))
        .ok()
        .and_then(|rel| rel.iter().next())
        .map(|user| user.to_string_lossy().into_owned())
        .with_context(|| format!("{} isn't in a home folder", source.display()))
}

#[cfg(test)]
#[path = "../tests/homes.rs"]
mod tests;
