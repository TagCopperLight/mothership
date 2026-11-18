mod account;
mod etc;
pub mod files;
mod home;
mod packages;
mod services;
mod settings;
pub mod tools;

use crate::{cmd, edit::Add, fleet::Fleet, pacman::Db, preview::Preview, revert::Revert, user};
use anyhow::{Result, bail};
use std::{
    collections::BTreeSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    thread::{self, ScopedJoinHandle},
};

/// Declared in report order.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Package,
    Tool,
    File,
    Home,
    Etc,
    Service,
    UserService,
    Account,
    Dconf,
    Pref,
}

impl Kind {
    const ALL: [Kind; 10] = [
        Kind::Package,
        Kind::Tool,
        Kind::File,
        Kind::Home,
        Kind::Etc,
        Kind::Service,
        Kind::UserService,
        Kind::Account,
        Kind::Dconf,
        Kind::Pref,
    ];

    /// The word ignore.txt uses.
    fn name(self) -> &'static str {
        match self {
            Kind::Package => "package",
            Kind::Tool => "tool",
            Kind::File => "file",
            Kind::Home => "home",
            Kind::Etc => "etc",
            Kind::Service => "service",
            Kind::UserService => "user-service",
            Kind::Account => "account",
            Kind::Dconf => "dconf",
            Kind::Pref => "pref",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Kind::Package => "Packages",
            Kind::Tool => "Tools outside pacman",
            Kind::File => "Files chezmoi tracks",
            Kind::Home => "Untracked config in ~",
            Kind::Etc => "/etc",
            Kind::Service => "System services",
            Kind::UserService => "User services",
            Kind::Account => "Account",
            Kind::Dconf => "dconf settings",
            Kind::Pref => "Browser prefs",
        }
    }

    pub fn parse(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

pub struct Item {
    pub kind: Kind,
    pub subject: String,
    pub detail: String,
    pub secret: bool,
    /// None when the fleet has no place for it yet.
    pub add: Option<Add>,
    /// Why it has no add, when the fleet has a place for its kind but not for this one.
    pub no_add: Option<&'static str>,
    pub revert: Option<Revert>,
    pub preview: Preview,
}

impl Item {
    fn new(kind: Kind, subject: impl Into<String>, detail: impl Into<String>) -> Self {
        Item {
            kind,
            subject: subject.into(),
            detail: detail.into(),
            secret: false,
            add: None,
            no_add: None,
            revert: None,
            preview: Preview::None,
        }
    }

    fn reverting(mut self, revert: Revert) -> Self {
        self.revert = Some(revert);
        self
    }

    fn showing(mut self, preview: Preview) -> Self {
        self.preview = preview;
        self
    }

    fn with(mut self, add: Add) -> Self {
        self.add = Some(add);
        self
    }

    fn because(mut self, no_add: &'static str) -> Self {
        self.add = None;
        self.no_add = Some(no_add);
        self
    }
}

pub fn ignore_line(item: &Item) -> String {
    format!("{} {}", item.kind.name(), globset::escape(&item.subject))
}

pub struct Report {
    pub fleet: Fleet,
    pub shown: Vec<Item>,
    pub ignored: usize,
}

pub fn collect() -> Result<Report> {
    if !Path::new("/usr/bin/pacman").exists() {
        bail!("mothership only knows Arch machines for now");
    }
    // The checks mostly wait on programs or the disk, so they run at once. chezmoi status is the
    // slowest and needs nothing, so it starts first.
    let (fleet, items) = thread::scope(|s| {
        let files = s.spawn(files::scan);
        let fleet = s.spawn(Fleet::load);
        let db = s.spawn(Db::load);
        let managed = s.spawn(managed);
        let fleet = join(fleet)?;
        let (db, managed) = (join(db)?, join(managed)?);
        let mut items = thread::scope(|s| {
            let (fleet, db, managed) = (&fleet, &db, &managed);
            let checks = vec![
                s.spawn(move || system(fleet, db)),
                s.spawn(move || tools::scan(fleet)),
                s.spawn(move || home::scan(fleet, managed)),
                s.spawn(move || settings::scan(managed)),
            ];
            checks.into_iter().map(join).collect::<Result<Vec<_>>>()
        })?;
        items.push(join(files)?);
        Ok::<_, anyhow::Error>((fleet, items.into_iter().flatten().collect::<Vec<Item>>()))
    })?;

    let (ignored, mut shown): (Vec<Item>, Vec<Item>) = items
        .into_iter()
        .partition(|item| fleet.ignore.matches(item));
    shown.sort_by_key(|item| item.kind);
    Ok(Report {
        fleet,
        shown,
        ignored: ignored.len(),
    })
}

/// The checks apply acts on: what lives outside $HOME, apart from tools.
pub fn system(fleet: &Fleet, db: &Db) -> Result<Vec<Item>> {
    let ships = Ships::new(fleet, db)?;
    thread::scope(|s| {
        let ships = &ships;
        let checks = vec![
            s.spawn(|| packages::scan(fleet, db)),
            s.spawn(|| etc::scan(fleet, db)),
            s.spawn(move || services::scan(fleet, ships)),
            s.spawn(move || account::scan(fleet, ships)),
        ];
        let items = checks.into_iter().map(join).collect::<Result<Vec<_>>>()?;
        Ok(items.into_iter().flatten().collect())
    })
}

/// What a new machine of the type gets before its services and account are set: the files of
/// the packages it installs and their dependencies, its etc/, and chezmoi's files. Review only offers to declare
/// a unit, group or shell that comes from there, so apply can set it up on that machine too.
pub struct Ships<'a> {
    fleet: &'a Fleet,
    db: &'a Db,
    listed: BTreeSet<String>,
}

impl<'a> Ships<'a> {
    pub fn new(fleet: &'a Fleet, db: &'a Db) -> Result<Self> {
        Ok(Ships {
            fleet,
            db,
            listed: db.with_depends(&fleet.packages()),
        })
    }

    /// /bin is a link to /usr/bin, where packages put their files, so the folder is resolved.
    pub fn path(&self, path: &Path) -> Result<bool> {
        let resolved = match (path.parent(), path.file_name()) {
            (Some(dir), Some(name)) => match fs::canonicalize(dir) {
                Ok(dir) => dir.join(name),
                Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
                Err(e) => return Err(e.into()),
            },
            _ => return Ok(false),
        };
        if let Ok(rel) = resolved.strip_prefix("/etc")
            && self.fleet.etc.contains(rel)
        {
            return Ok(true);
        }
        if resolved.starts_with(user::home()) {
            // Fails for a file chezmoi doesn't manage.
            let file = resolved.to_string_lossy();
            return Ok(cmd::stdout("chezmoi", &["source-path", &file]).is_ok());
        }
        let file = resolved.to_string_lossy();
        Ok(self
            .db
            .packages
            .iter()
            .filter(|package| self.listed.contains(&package.name))
            .any(|package| package.files.iter().any(|f| *f == file)))
    }
}

fn join<T>(handle: ScopedJoinHandle<'_, Result<T>>) -> Result<T> {
    handle
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

fn managed() -> Result<Vec<PathBuf>> {
    let args = [
        "managed",
        "--path-style",
        "absolute",
        "--include",
        "files,symlinks",
    ];
    Ok(cmd::stdout("chezmoi", &args)?
        .lines()
        .map(PathBuf::from)
        .collect())
}

pub fn run() -> Result<()> {
    let report = collect()?;
    let shown: Vec<&Item> = report.shown.iter().collect();
    print(&shown, |item| {
        let secret = if item.secret { "  [secret]" } else { "" };
        format!("{}{secret}", item.detail)
    });
    println!(
        "{} differences, {} ignored by {}",
        report.shown.len(),
        report.ignored,
        crate::fleet::tilde(&report.fleet.ignore_path())
    );
    Ok(())
}

/// Grouped by kind, one line per item.
pub fn print(items: &[&Item], detail: impl Fn(&Item) -> String) {
    for kind in Kind::ALL {
        let group: Vec<&&Item> = items.iter().filter(|item| item.kind == kind).collect();
        if group.is_empty() {
            continue;
        }
        println!("{} ({})", kind.title(), group.len());
        let width = group
            .iter()
            .map(|item| item.subject.len())
            .max()
            .unwrap_or(0)
            .min(56);
        for item in group {
            println!("  {:width$}  {}", item.subject, detail(item));
        }
        println!();
    }
}

/// A guess, so the report can warn before anything gets committed in clear. Review never shows
/// a secret's contents, since its screen can end up in logs and transcripts.
fn looks_secret(mut item: Item, path: &Path) -> Item {
    item.secret = guess_secret(path);
    if item.secret {
        item.preview = Preview::None;
        return item.because("secrets wait for the secrets system");
    }
    item
}

fn guess_secret(path: &Path) -> bool {
    let text = path.to_string_lossy().to_lowercase();
    if text.contains("/.ssh") || text.contains("/.gnupg") || text.starts_with("/etc/ssh/") {
        return true;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if ["key", "token", "secret", "credential", "password", ".pem"]
        .iter()
        .any(|word| name.contains(word))
    {
        return true;
    }
    fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() < 64 * 1024)
        && fs::read(path).is_ok_and(|bytes| bytes.windows(11).any(|w| w == b"PRIVATE KEY"))
}

#[cfg(test)]
#[path = "../../tests/scan.rs"]
mod tests;
