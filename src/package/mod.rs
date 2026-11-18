mod dpkg;
mod pacman;

use crate::fleet::Fleet;
use anyhow::{Context, Result, bail};
use md5::{Digest, Md5};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    thread,
};

const OS_RELEASE: &str = "/etc/os-release";

/// The family the machine's system belongs to, which decides its package manager.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Distro {
    Arch,
    Debian,
}

impl Distro {
    pub fn here() -> Result<Distro> {
        let text =
            fs::read_to_string(OS_RELEASE).with_context(|| format!("reading {OS_RELEASE}"))?;
        Distro::of(&text)
    }

    /// Ubuntu names Debian in ID_LIKE.
    fn of(os_release: &str) -> Result<Distro> {
        let field = |name: &str| {
            os_release
                .lines()
                .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
                .map(|value| value.trim_matches('"'))
                .unwrap_or_default()
        };
        let family = format!("{} {}", field("ID"), field("ID_LIKE"));
        let family: Vec<&str> = family.split_whitespace().collect();
        if family.contains(&"arch") {
            Ok(Distro::Arch)
        } else if family.contains(&"debian") {
            Ok(Distro::Debian)
        } else {
            bail!("mothership knows Arch and Debian machines, and {OS_RELEASE} names neither")
        }
    }

    /// Why apply can't install a package, or nothing when it can.
    pub fn cannot_install(self, fleet: &Fleet, name: &str) -> Result<Option<String>> {
        match self {
            Distro::Arch => pacman::cannot_install(fleet, name),
            Distro::Debian => dpkg::cannot_install(name),
        }
    }

    /// The package manager's own config, which says where packages come from, so apply writes
    /// it before installing.
    pub fn configures(self, path: &Path) -> bool {
        let config: &[&str] = match self {
            Distro::Arch => &["/etc/pacman.conf", "/etc/pacman.d"],
            Distro::Debian => &["/etc/apt"],
        };
        config.iter().any(|dir| path.starts_with(dir))
    }

    pub fn install(self, fleet: &Fleet, names: &[&str]) -> Result<()> {
        match self {
            Distro::Arch => pacman::install(fleet, names),
            Distro::Debian => dpkg::install(names),
        }
    }

    /// Removes the packages nothing else needs, with the dependencies only they needed, and
    /// keeps the others as dependencies.
    pub fn uninstall(self, names: &[&str]) -> Result<()> {
        match self {
            Distro::Arch => pacman::uninstall(names),
            Distro::Debian => dpkg::uninstall(names),
        }
    }
}

/// Read directly: asking the package manager for edited config files checks every file of
/// every package, which takes half a minute.
pub struct Db {
    pub packages: Vec<Package>,
}

pub struct Package {
    pub name: String,
    pub description: String,
    pub explicit: bool,
    /// Folders end with a `/`.
    pub files: Vec<String>,
    /// The config files the package ships, with the md5 of the version it shipped.
    pub backups: Vec<(PathBuf, String)>,
    pub depends: Vec<String>,
    pub provides: Vec<String>,
    pub archive: Archive,
}

/// Where the package manager keeps the package it installed, which holds the original config.
#[derive(Clone)]
pub struct Archive {
    pub path: PathBuf,
    pub distro: Distro,
}

impl Archive {
    pub fn shipped(&self, file: &Path) -> Result<Vec<u8>> {
        let member = file.strip_prefix("/")?;
        match self.distro {
            Distro::Arch => pacman::shipped(&self.path, member),
            Distro::Debian => dpkg::shipped(&self.path, member),
        }
    }
}

impl Db {
    pub fn load() -> Result<Self> {
        let packages = match Distro::here()? {
            Distro::Arch => pacman::load()?,
            Distro::Debian => dpkg::load()?,
        };
        Ok(Db { packages })
    }

    pub fn with_depends(&self, names: &BTreeSet<String>) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        let mut todo: Vec<&str> = names.iter().map(String::as_str).collect();
        while let Some(name) = todo.pop() {
            let Some(package) = self.packages.iter().find(|p| p.name == name).or_else(|| {
                self.packages
                    .iter()
                    .find(|p| p.provides.iter().any(|n| n == name))
            }) else {
                continue;
            };
            if found.insert(package.name.clone()) {
                todo.extend(package.depends.iter().map(String::as_str));
            }
        }
        found
    }
}

/// Loads each item on its own thread share, keeping the items' order.
fn in_parallel<T: Sync, U: Send>(
    items: &[T],
    load: impl Fn(&T) -> Result<U> + Sync,
) -> Result<Vec<U>> {
    let threads = thread::available_parallelism().map_or(4, usize::from);
    let load = &load;
    thread::scope(|s| {
        let parts: Vec<_> = items
            .chunks(items.len().div_ceil(threads).max(1))
            .map(|chunk| s.spawn(move || chunk.iter().map(load).collect::<Result<Vec<_>>>()))
            .collect();
        let parts = parts
            .into_iter()
            .map(|part| part.join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
            .collect::<Result<Vec<_>>>()?;
        Ok(parts.into_iter().flatten().collect())
    })
}

pub fn md5(bytes: &[u8]) -> String {
    Md5::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
#[path = "../../tests/package.rs"]
mod tests;
