use super::{ETC_DIR, TYPES, USERS, type_dir, types, users};
use crate::{home::chezmoi, secrets, services, util::walk};
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

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Reader {
    Root(String),
    Home(Home),
}

impl Reader {
    pub fn dir(&self, fleet_dir: &Path) -> PathBuf {
        match self {
            Reader::Root(type_name) => type_dir(fleet_dir, type_name),
            Reader::Home(home) => home.dir(fleet_dir),
        }
    }

    pub fn type_name(&self) -> &str {
        match self {
            Reader::Root(type_name) => type_name,
            Reader::Home(home) => &home.type_name,
        }
    }
}

impl fmt::Display for Reader {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Reader::Root(type_name) => write!(f, "{type_name}/root"),
            Reader::Home(home) => home.fmt(f),
        }
    }
}

pub fn readers(fleet_dir: &Path) -> Result<Vec<Reader>> {
    let mut all = Vec::new();
    for type_name in types(fleet_dir)? {
        all.push(Reader::Root(type_name.clone()));
        for user in users(&type_dir(fleet_dir, &type_name))? {
            all.push(Reader::Home(Home {
                type_name: type_name.clone(),
                user,
            }));
        }
    }
    Ok(all)
}

const ROOT_PLACES: [&str; 2] = [ETC_DIR, services::SERVICES];

pub struct Shared {
    pub rel: PathBuf,
    pub readers: Vec<Reader>,
}

pub fn check(fleet_dir: &Path) -> Result<()> {
    for link in links(fleet_dir)? {
        let file = target(fleet_dir, &link)?;
        let secret = |path: &Path| path.to_string_lossy().ends_with(secrets::AGE);
        if matches!(owner(fleet_dir, &link)?, Reader::Root(_)) && !secret(&file) {
            bail!(
                "{} leads to a file in clear: root links only shared secrets",
                link.strip_prefix(fleet_dir).unwrap_or(&link).display()
            );
        }
    }
    for home in all(fleet_dir)? {
        let source = home.source(fleet_dir);
        for rel in walk::files(&source)? {
            check_private(fleet_dir, &source, &rel)?;
        }
    }
    Ok(())
}

fn check_private(fleet_dir: &Path, source: &Path, rel: &Path) -> Result<()> {
    if rel.starts_with(SECRETS) {
        return Ok(());
    }
    let target = chezmoi::target_of(rel);
    if target.private {
        return Ok(());
    }
    let path = source.join(rel);
    let what = if target.encrypted {
        "is a secret"
    } else if path.to_string_lossy().ends_with(chezmoi::TEMPLATE)
        && String::from_utf8_lossy(&fs::read(&path)?).contains(&format!("{SECRETS}/"))
    {
        "reads a secret"
    } else {
        return Ok(());
    };
    bail!(
        "{} {what}, so only its owner may read it: add `private_` to its name",
        path.strip_prefix(fleet_dir).unwrap_or(&path).display()
    )
}

pub fn owners(fleet_dir: &Path, source: &Path) -> Result<Vec<Reader>> {
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
    let mut found: BTreeMap<PathBuf, Vec<Reader>> =
        files.into_iter().map(|rel| (rel, Vec::new())).collect();
    for link in links(fleet_dir)? {
        let rel = target(fleet_dir, &link)?
            .strip_prefix(&canonical)?
            .to_path_buf();
        if let Some(readers) = found.get_mut(&rel) {
            readers.push(owner(fleet_dir, &link)?);
        }
    }
    Ok(found
        .into_iter()
        .map(|(rel, mut readers)| {
            readers.sort();
            readers.dedup();
            Shared { rel, readers }
        })
        .collect())
}

pub fn shown(rel: &Path) -> String {
    if rel.starts_with(SECRETS) {
        return rel.display().to_string();
    }
    format!("~/{}", chezmoi::target_of(rel).path.display())
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
    Ok(readers(fleet_dir)?
        .into_iter()
        .filter_map(|reader| match reader {
            Reader::Home(home) => Some(home),
            Reader::Root(_) => None,
        })
        .collect())
}

fn links(fleet_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut links = Vec::new();
    for home in all(fleet_dir)? {
        links.extend(links_in(&home.source(fleet_dir), |_| true)?);
    }
    for type_name in types(fleet_dir)? {
        for place in ROOT_PLACES {
            let dir = type_dir(fleet_dir, &type_name).join(place);
            links.extend(links_in(&dir, |path| {
                path.to_string_lossy().ends_with(secrets::AGE)
            })?);
        }
    }
    Ok(links)
}

fn links_in(dir: &Path, wanted: impl Fn(&Path) -> bool) -> Result<Vec<PathBuf>> {
    let mut links = Vec::new();
    for rel in walk::files(dir)? {
        let path = dir.join(rel);
        if wanted(&path) && fs::symlink_metadata(&path)?.is_symlink() {
            links.push(path);
        }
    }
    Ok(links)
}

pub fn owner(fleet_dir: &Path, source: &Path) -> Result<Reader> {
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
        [type_name, USERS, user, HOME] => Ok(Reader::Home(Home {
            type_name: type_name.to_owned(),
            user: user.to_owned(),
        })),
        [type_name, place, ..] if ROOT_PLACES.contains(&place) => {
            Ok(Reader::Root(type_name.to_owned()))
        }
        _ => bail!("{} isn't in a home or root's folders", source.display()),
    }
}

#[cfg(test)]
#[path = "../../tests/fleet/homes.rs"]
mod tests;
