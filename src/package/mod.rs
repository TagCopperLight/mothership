mod apt;
pub mod aur;
mod dpkg;
mod pacman;

pub use apt::keep_only;

use crate::system::user::User;
use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
};

const OS_RELEASE: &str = "/etc/os-release";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Distro {
    Arch,
    Debian,
}

pub struct Aur<'a> {
    pub helper: Option<aur::Helper>,
    pub admin: &'a User,
}

impl Distro {
    pub fn here() -> Result<Distro> {
        let text =
            fs::read_to_string(OS_RELEASE).with_context(|| format!("reading {OS_RELEASE}"))?;
        Distro::of(&text)
    }

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

    pub fn cannot_install(self, aur: &Aur, name: &str) -> Result<Option<String>> {
        match self {
            Distro::Arch => pacman::cannot_install(aur, name),
            Distro::Debian => apt::cannot_install(name),
        }
    }

    pub fn configures(self, path: &Path) -> bool {
        let config: &[&str] = match self {
            Distro::Arch => &["/etc/pacman.conf", "/etc/pacman.d"],
            Distro::Debian => &["/etc/apt"],
        };
        config.iter().any(|dir| path.starts_with(dir))
    }

    pub fn ssh_package(self) -> &'static str {
        match self {
            Distro::Arch => "openssh",
            Distro::Debian => "openssh-client",
        }
    }

    pub fn bootstrap(self, names: &[&str]) -> Result<()> {
        match self {
            Distro::Arch => pacman::bootstrap(names),
            Distro::Debian => apt::install(names),
        }
    }

    pub fn install(self, aur: &Aur, names: &[&str]) -> Result<()> {
        match self {
            Distro::Arch => pacman::install(aur, names)?,
            Distro::Debian => apt::install(names)?,
        }
        Db::load()?.require_installed(names)
    }

    pub fn uninstall(self, listed: &BTreeSet<String>, names: &[&str]) -> Result<()> {
        let db = Db::load()?;
        let keep: Vec<&str> = db
            .packages
            .iter()
            .map(|package| package.name.as_str())
            .filter(|name| listed.contains(*name))
            .collect();
        let removed = match self {
            Distro::Arch => pacman::uninstall(&keep, names)?,
            Distro::Debian => apt::uninstall(&db, &keep, names)?,
        };
        for name in names.iter().filter(|name| !removed.contains(**name)) {
            println!("{}", stays(name, &db.needed_by(name)));
        }
        Ok(())
    }
}

fn or_restore<T>(result: Result<T>, restore: impl FnOnce() -> Result<()>) -> Result<T> {
    result.map_err(|error| match restore() {
        Ok(()) => error,
        Err(restore) => error.context(format!(
            "and marking them as installed by hand again failed: {restore:#}"
        )),
    })
}

fn stays(name: &str, users: &[&str]) -> String {
    if users.is_empty() {
        format!("{name} stays installed, as a dependency of another package")
    } else {
        format!("{name} stays installed, as {} needs it", users.join(", "))
    }
}

pub struct Db {
    pub distro: Distro,
    pub packages: Vec<Package>,
    index: HashMap<String, usize>,
}

pub struct Package {
    pub name: String,
    pub description: String,
    pub explicit: bool,
    pub required: bool,
    pub files: Vec<String>,
    pub backups: Vec<Backup>,
    pub depends: Vec<String>,
    pub provides: Vec<String>,
    pub groups: Vec<String>,
    pub archive: Archive,
}

pub struct Backup {
    pub path: PathBuf,
    pub md5: String,
    pub origin: Origin,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    Archive,
    Script,
}

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

    pub fn cached(&self) -> bool {
        self.path.exists()
    }
}

impl Db {
    pub fn load() -> Result<Self> {
        let distro = Distro::here()?;
        let packages = match distro {
            Distro::Arch => pacman::load()?,
            Distro::Debian => dpkg::load()?,
        };
        Ok(Db::new(distro, packages))
    }

    pub fn new(distro: Distro, packages: Vec<Package>) -> Self {
        let mut index = HashMap::new();
        for (i, package) in packages.iter().enumerate() {
            for name in &package.provides {
                index.entry(name.clone()).or_insert(i);
            }
        }
        for (i, package) in packages.iter().enumerate() {
            index.insert(package.name.clone(), i);
        }
        Db {
            distro,
            packages,
            index,
        }
    }

    fn require_installed(&self, names: &[&str]) -> Result<()> {
        for name in names {
            if self
                .resolve(name)
                .is_none_or(|package| package.name != *name)
            {
                bail!(
                    "{name} isn't the name of an installed package (a group, a virtual name or a version?): list the real package names instead"
                );
            }
        }
        Ok(())
    }

    pub fn resolve(&self, name: &str) -> Option<&Package> {
        self.index.get(name).map(|&i| &self.packages[i])
    }

    pub fn with_depends(&self, names: &BTreeSet<String>) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        let mut todo: Vec<&str> = names.iter().map(String::as_str).collect();
        while let Some(name) = todo.pop() {
            if let Some(package) = self.resolve(name)
                && found.insert(package.name.clone())
            {
                todo.extend(package.depends.iter().map(String::as_str));
            }
        }
        found
    }

    pub fn needing(&self, needed: &BTreeSet<String>) -> BTreeSet<String> {
        let users = self.users();
        let mut seen = vec![false; self.packages.len()];
        let mut todo: Vec<usize> = needed
            .iter()
            .filter_map(|name| self.index.get(name).copied())
            .collect();
        while let Some(i) = todo.pop() {
            if !std::mem::replace(&mut seen[i], true) {
                todo.extend(&users[i]);
            }
        }
        seen.iter()
            .zip(&self.packages)
            .filter(|(seen, _)| **seen)
            .map(|(_, package)| package.name.clone())
            .collect()
    }

    fn needed_by(&self, name: &str) -> Vec<&str> {
        let Some(&i) = self.index.get(name) else {
            return Vec::new();
        };
        self.users()[i]
            .iter()
            .map(|&user| self.packages[user].name.as_str())
            .collect()
    }

    fn users(&self) -> Vec<Vec<usize>> {
        let mut users = vec![Vec::new(); self.packages.len()];
        for (i, package) in self.packages.iter().enumerate() {
            for depend in &package.depends {
                if let Some(&j) = self.index.get(depend)
                    && j != i
                    && users[j].last() != Some(&i)
                {
                    users[j].push(i);
                }
            }
        }
        users
    }
}

#[cfg(test)]
#[path = "../../tests/package/mod.rs"]
mod tests;
