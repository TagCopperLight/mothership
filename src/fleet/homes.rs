use super::{TYPES, USERS, type_dir, types};
use crate::{chezmoi, walk};
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeMap,
    fmt, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

pub const HOME: &str = "home";
pub const SECRETS: &str = ".secrets";
pub const KEEP: &str = ".keep";
const SHARED: &str = "shared";

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Home {
    pub type_name: String,
    pub user: String,
}

impl Home {
    pub fn dir(&self, fleet_dir: &Path) -> PathBuf {
        type_dir(fleet_dir, &self.type_name)
            .join(USERS)
            .join(&self.user)
    }

    pub fn source(&self, fleet_dir: &Path) -> PathBuf {
        self.dir(fleet_dir).join(HOME)
    }
}

impl fmt::Display for Home {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}/{}", self.type_name, self.user)
    }
}

pub struct Shared {
    pub rel: PathBuf,
    pub homes: Vec<Home>,
}

pub fn check(fleet_dir: &Path) -> Result<()> {
    for link in links(fleet_dir)? {
        target(fleet_dir, &link)?;
    }
    Ok(())
}

pub fn owners(fleet_dir: &Path, source: &Path) -> Result<Vec<Home>> {
    let is_link = fs::symlink_metadata(source).is_ok_and(|meta| meta.is_symlink());
    if !is_link {
        return Ok(vec![owner(fleet_dir, source)?]);
    }
    let shared = target(fleet_dir, source)?;
    let mut homes = Vec::new();
    for link in links(fleet_dir)? {
        if target(fleet_dir, &link)? == shared {
            homes.push(owner(fleet_dir, &link)?);
        }
    }
    homes.sort();
    homes.dedup();
    Ok(homes)
}

pub fn shared(fleet_dir: &Path) -> Result<Vec<Shared>> {
    let dir = fleet_dir.join(SHARED);
    let files = walk::files(&dir)?;
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let canonical = dir.canonicalize()?;
    let mut found: BTreeMap<PathBuf, Vec<Home>> =
        files.into_iter().map(|rel| (rel, Vec::new())).collect();
    for link in links(fleet_dir)? {
        let rel = target(fleet_dir, &link)?
            .strip_prefix(&canonical)?
            .to_path_buf();
        if let Some(homes) = found.get_mut(&rel) {
            homes.push(owner(fleet_dir, &link)?);
        }
    }
    Ok(found
        .into_iter()
        .map(|(rel, mut homes)| {
            homes.sort();
            homes.dedup();
            Shared { rel, homes }
        })
        .collect())
}

pub fn shown(rel: &Path) -> String {
    if rel.starts_with(SECRETS) {
        return rel.display().to_string();
    }
    let (path, _) = chezmoi::target_of(rel);
    format!("~/{}", path.display())
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

fn all(fleet_dir: &Path) -> Result<Vec<Home>> {
    let mut homes = Vec::new();
    for type_name in types(fleet_dir)? {
        for dir in walk::children(&type_dir(fleet_dir, &type_name).join(USERS))? {
            if !dir.is_dir() {
                continue;
            }
            let user = dir
                .file_name()
                .and_then(|n| n.to_str())
                .with_context(|| format!("{} is not a user name", dir.display()))?;
            homes.push(Home {
                type_name: type_name.clone(),
                user: user.to_owned(),
            });
        }
    }
    Ok(homes)
}

fn links(fleet_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut links = Vec::new();
    for home in all(fleet_dir)? {
        let source = home.source(fleet_dir);
        for rel in walk::files(&source)? {
            let path = source.join(rel);
            if fs::symlink_metadata(&path)?.is_symlink() {
                links.push(path);
            }
        }
    }
    Ok(links)
}

fn owner(fleet_dir: &Path, source: &Path) -> Result<Home> {
    let parts: Vec<&str> = source
        .strip_prefix(fleet_dir.join(TYPES))
        .ok()
        .into_iter()
        .flat_map(Path::iter)
        .take(4)
        .map(|part| part.to_str())
        .collect::<Option<_>>()
        .unwrap_or_default();
    match parts[..] {
        [type_name, USERS, user, HOME] => Ok(Home {
            type_name: type_name.to_owned(),
            user: user.to_owned(),
        }),
        _ => bail!("{} isn't in a home folder", source.display()),
    }
}

#[cfg(test)]
#[path = "../../tests/homes.rs"]
mod tests;
