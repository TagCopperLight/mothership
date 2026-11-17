use anyhow::{Context, Result};
use std::{
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
    arch: String,
    pub description: String,
    pub explicit: bool,
    /// Absolute paths. Folders end with `/`.
    pub files: Vec<String>,
    /// Config files pacman keeps on upgrade, with the MD5 of the version it installed.
    pub backups: Vec<(PathBuf, String)>,
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

fn section<'a>(text: &'a str, name: &str) -> impl Iterator<Item = &'a str> {
    let header = format!("%{name}%");
    text.lines()
        .skip_while(move |line| *line != header)
        .skip(1)
        .take_while(|line| !line.is_empty())
}
