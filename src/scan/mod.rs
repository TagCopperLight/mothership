mod account;
mod etc;
pub mod files;
mod home;
mod packages;
mod services;
mod settings;
pub mod tools;

use crate::{
    chezmoi,
    edit::Add,
    fleet::{Fleet, Member},
    ignore::{Level, Paths},
    pacman::Db,
    preview::Preview,
    revert::Revert,
};
use anyhow::{Result, bail};
use std::{
    collections::BTreeSet,
    fs,
    io::ErrorKind,
    os::unix::fs::PermissionsExt,
    path::Path,
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

    /// The type's own kinds live outside the homes, the rest belong to each user.
    pub fn level(self) -> Level {
        match self {
            Kind::Package | Kind::Etc | Kind::Service => Level::Machine,
            _ => Level::User,
        }
    }

    /// The word ignore.txt uses.
    pub fn name(self) -> &'static str {
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

/// Whether an item is a secret, and why.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Secret {
    No,
    /// Only its owner can read it, and not-secret.txt doesn't clear it.
    Guessed,
    /// The fleet keeps it encrypted.
    Fleet,
    /// The fleet keeps it encrypted, and this machine's key can't open it.
    Locked,
}

pub struct Item {
    pub kind: Kind,
    /// The index of the type's user it belongs to, None for the machine's own.
    pub member: Option<usize>,
    pub subject: String,
    pub detail: String,
    pub secret: Secret,
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
            member: None,
            subject: subject.into(),
            detail: detail.into(),
            secret: Secret::No,
            add: None,
            no_add: None,
            revert: None,
            preview: Preview::None,
        }
    }

    fn locked(kind: Kind, subject: impl Into<String>) -> Self {
        let mut item = Item::new(kind, subject, "this machine's key can't open it");
        item.secret = Secret::Locked;
        item
    }

    /// Review never shows a secret's contents, since its screen can end up in logs and
    /// transcripts, and adds it encrypted.
    fn secret(mut self, secret: Secret) -> Self {
        self.secret = secret;
        self.preview = Preview::None;
        self.add = self.add.map(|add| add.encrypted().unwrap_or(add));
        self
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

/// The ignore line, with the user it belongs to.
pub fn key(fleet: &Fleet, item: &Item) -> String {
    match item.member {
        None => ignore_line(item),
        Some(i) => format!("{}: {}", fleet.members[i].name, ignore_line(item)),
    }
}

/// The kind's title, with the user an item of theirs belongs to.
pub fn title(fleet: &Fleet, item: &Item) -> String {
    match item.member {
        None => item.kind.title().to_owned(),
        Some(i) => format!("{}, {}", item.kind.title(), fleet.members[i].name),
    }
}

pub fn ignored(fleet: &Fleet, item: &Item) -> bool {
    match item.member {
        None => fleet.ignore.matches(item),
        Some(i) => fleet.members[i].ignore.matches(item),
    }
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
    let (fleet, db) = thread::scope(|s| {
        let fleet = s.spawn(Fleet::load);
        let db = s.spawn(Db::load);
        Ok::<_, anyhow::Error>((join(fleet)?, join(db)?))
    })?;
    let items = checks(&fleet, &db, Checks::All)?;
    let (ignored, mut shown): (Vec<Item>, Vec<Item>) =
        items.into_iter().partition(|item| ignored(&fleet, item));
    shown.sort_by_key(|item| (item.member, item.kind));
    Ok(Report {
        fleet,
        shown,
        ignored: ignored.len(),
    })
}

/// Apply acts on what it can bring back: everything but untracked config and settings.
#[derive(Clone, Copy, PartialEq)]
pub enum Checks {
    All,
    Apply,
}

/// The machine's checks, then each user's. They mostly wait on programs or the disk, so they
/// run at once.
pub fn checks(fleet: &Fleet, db: &Db, which: Checks) -> Result<Vec<Item>> {
    let ships = Ships::new(fleet, db)?;
    thread::scope(|s| {
        let ships = &ships;
        let mut handles = vec![
            s.spawn(move || packages::scan(fleet, db)),
            s.spawn(move || etc::scan(fleet, db)),
            s.spawn(move || services::scan(fleet, ships)),
        ];
        for i in 0..fleet.members.len() {
            handles.push(s.spawn(move || member(fleet, ships, i, which)));
        }
        let items = handles.into_iter().map(join).collect::<Result<Vec<_>>>()?;
        Ok(items.into_iter().flatten().collect())
    })
}

/// A user the machine lacks has nothing else to check, and a user whose chezmoi doesn't read
/// the fleet has no files to compare.
fn member(fleet: &Fleet, ships: &Ships, i: usize, which: Checks) -> Result<Vec<Item>> {
    let member = &fleet.members[i];
    let mut items = Vec::new();
    match &member.user {
        None => items.push(
            Item::new(
                Kind::Account,
                format!("user {}", member.name),
                "declared, missing",
            )
            .reverting(Revert::CreateUser),
        ),
        Some(user) => {
            items.extend(account::scan(member, ships)?);
            items.extend(services::scan_user(member, ships)?);
            items.extend(tools::scan(member)?);
            if member.chezmoi {
                let managed = chezmoi::managed(user, "files,symlinks")?;
                items.extend(files::scan(member, &managed)?);
                if which == Checks::All {
                    items.extend(home::scan(fleet, member, &managed)?);
                    items.extend(settings::scan(member, &managed)?);
                }
            } else {
                items.push(
                    Item::new(Kind::File, "chezmoi", "doesn't read the fleet")
                        .reverting(Revert::InitChezmoi),
                );
            }
        }
    }
    for item in &mut items {
        item.member = Some(i);
    }
    Ok(items)
}

/// What a new machine of the type gets before its services and account are set: the files of
/// the packages it installs and their dependencies, its etc/, and chezmoi's files. Review only
/// offers to declare a unit, group or shell that comes from there, so apply can set it up on
/// that machine too.
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
    /// A file in a user's home ships when their chezmoi writes it.
    pub fn path(&self, path: &Path, member: Option<&Member>) -> Result<bool> {
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
        if let Some(user) = member.and_then(|m| m.user.as_ref())
            && resolved.starts_with(&user.home)
        {
            // Fails for a file chezmoi doesn't manage.
            return Ok(chezmoi::source_path(user, &resolved).is_ok());
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

pub fn run() -> Result<()> {
    let report = collect()?;
    let shown: Vec<&Item> = report.shown.iter().collect();
    print(&report.fleet, &shown, |item| {
        format!("{}{}", item.detail, item.secret.mark())
    });
    println!(
        "{} differences, {} ignored",
        report.shown.len(),
        report.ignored
    );
    Ok(())
}

/// Grouped by user and kind, one line per item. `items` come sorted that way.
pub fn print(fleet: &Fleet, items: &[&Item], detail: impl Fn(&Item) -> String) {
    for group in items.chunk_by(|a, b| (a.member, a.kind) == (b.member, b.kind)) {
        println!("{} ({})", title(fleet, group[0]), group.len());
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

impl Secret {
    pub fn mark(self) -> &'static str {
        if self == Secret::No { "" } else { "  [secret]" }
    }
}

/// A guess, so a secret is never added in clear by mistake: a file only its owner can read, or
/// a folder holding one. A folder it can't open counts too. not-secret.txt clears the guess.
fn looks_secret(not_secret: &Paths, item: Item, path: &Path) -> Item {
    if !not_secret.matches(&item.subject) && guess_secret(path) {
        item.secret(Secret::Guessed)
    } else {
        item
    }
}

fn guess_secret(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| {
        if meta.is_dir() {
            holds_owner_only(path)
        } else {
            owner_only(&meta)
        }
    })
}

fn owner_only(meta: &fs::Metadata) -> bool {
    meta.is_file() && meta.permissions().mode() & 0o044 == 0
}

/// Stops at the first one, since a big folder of caches holds none.
fn holds_owner_only(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return true;
    };
    entries.into_iter().any(|entry| {
        let Ok((path, Ok(meta))) = entry.map(|e| (e.path(), e.metadata())) else {
            return true;
        };
        if meta.is_dir() {
            holds_owner_only(&path)
        } else {
            owner_only(&meta)
        }
    })
}

#[cfg(test)]
#[path = "../../tests/scan.rs"]
mod tests;
