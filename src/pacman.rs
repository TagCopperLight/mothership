use crate::cmd::{self, As};
use anyhow::{Context, Result};
use md5::{Digest, Md5};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    thread,
};

const LOCAL: &str = "/var/lib/pacman/local";
const CACHE: &str = "/var/cache/pacman/pkg";

/// Read directly: asking pacman for edited config files (`pacman -Qkk`) checks every file of
/// every package, which takes half a minute.
pub struct Db {
    pub packages: Vec<Package>,
}

pub struct Package {
    pub name: String,
    pub version: String,
    pub arch: String,
    pub description: String,
    pub explicit: bool,
    /// Absolute paths. Folders end with `/`.
    pub files: Vec<String>,
    /// Config files pacman keeps on upgrade, with the MD5 of the version it installed.
    pub backups: Vec<(PathBuf, String)>,
    /// Names without their version constraints, like `sh` from `sh>=5`.
    pub depends: Vec<String>,
    pub provides: Vec<String>,
}

impl Db {
    pub fn load() -> Result<Self> {
        let mut dirs = Vec::new();
        for entry in fs::read_dir(LOCAL).with_context(|| format!("reading {LOCAL}"))? {
            let dir = entry?.path();
            if dir.is_dir() {
                dirs.push(dir);
            }
        }
        // The folder's order depends on the filesystem, and the scan reports in this order.
        dirs.sort();
        let threads = thread::available_parallelism().map_or(4, usize::from);
        let packages = thread::scope(|s| {
            let parts: Vec<_> = dirs
                .chunks(dirs.len().div_ceil(threads).max(1))
                .map(|chunk| {
                    s.spawn(move || {
                        chunk
                            .iter()
                            .map(|dir| Package::load(dir))
                            .collect::<Result<Vec<_>>>()
                    })
                })
                .collect();
            parts
                .into_iter()
                .map(|part| part.join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
                .collect::<Result<Vec<_>>>()
        })?;
        Ok(Db {
            packages: packages.into_iter().flatten().collect(),
        })
    }
}

impl Db {
    /// The named packages and all they depend on, as installed here.
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

fn names(desc: &str, field: &str) -> Vec<String> {
    section(desc, field)
        .filter_map(|dep| dep.split(['<', '>', '=']).next())
        .map(str::to_owned)
        .collect()
}

impl Package {
    fn load(dir: &Path) -> Result<Self> {
        let read = |name| {
            let path = dir.join(name);
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))
        };
        let desc = read("desc")?;
        let files = read("files")?;
        let field = |name| section(&desc, name).next().unwrap_or_default().to_owned();
        Ok(Package {
            name: field("NAME"),
            version: field("VERSION"),
            arch: field("ARCH"),
            description: field("DESC"),
            // pacman writes REASON only for dependencies.
            explicit: section(&desc, "REASON").next() != Some("1"),
            files: section(&files, "FILES").map(|f| format!("/{f}")).collect(),
            depends: names(&desc, "DEPENDS"),
            provides: names(&desc, "PROVIDES"),
            backups: section(&files, "BACKUP")
                .filter_map(|line| line.split_once('\t'))
                .map(|(path, md5)| (Path::new("/").join(path), md5.to_owned()))
                .collect(),
        })
    }

    pub fn archive(&self) -> PathBuf {
        Path::new(CACHE).join(format!(
            "{}-{}-{}.pkg.tar.zst",
            self.name, self.version, self.arch
        ))
    }
}

/// The names pacman's sync databases have.
pub fn in_repos() -> Result<BTreeSet<String>> {
    Ok(cmd::stdout(As::Root, "pacman", &["-Slq"])?
        .lines()
        .map(str::to_owned)
        .collect())
}

/// The file as the package shipped it, from the archive in pacman's cache.
pub fn shipped(archive: &Path, path: &Path) -> Result<Vec<u8>> {
    let member = path.strip_prefix("/")?.to_string_lossy().into_owned();
    cmd::bytes(
        As::Root,
        "bsdtar",
        &["-xOf", &archive.to_string_lossy(), &member],
    )
}

/// pacman records config files by their MD5.
pub fn md5(bytes: &[u8]) -> String {
    Md5::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn section<'a>(text: &'a str, name: &str) -> impl Iterator<Item = &'a str> {
    let header = format!("%{name}%");
    text.lines()
        .skip_while(move |line| *line != header)
        .skip(1)
        .take_while(|line| !line.is_empty())
}

#[cfg(test)]
#[path = "../tests/pacman.rs"]
mod tests;
