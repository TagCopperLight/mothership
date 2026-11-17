mod account;
mod etc;
mod files;
mod home;
mod packages;
mod services;
mod settings;
mod tools;

use crate::{cmd, edit::Add, fleet::Fleet};
use anyhow::{Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
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

/// One difference between this machine and its device type.
pub struct Item {
    pub kind: Kind,
    pub subject: String,
    pub detail: String,
    pub secret: bool,
    /// None when the fleet has no place for it yet.
    pub add: Option<Add>,
}

impl Item {
    fn new(kind: Kind, subject: impl Into<String>, detail: impl Into<String>) -> Self {
        Item {
            kind,
            subject: subject.into(),
            detail: detail.into(),
            secret: false,
            add: None,
        }
    }

    fn with(mut self, add: Add) -> Self {
        self.add = Some(add);
        self
    }
}

/// The line in ignore.txt that matches exactly this item.
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
    let fleet = Fleet::load()?;
    let managed: Vec<PathBuf> = cmd::stdout(
        "chezmoi",
        &[
            "managed",
            "--path-style",
            "absolute",
            "--include",
            "files,symlinks",
        ],
    )?
    .lines()
    .map(PathBuf::from)
    .collect();

    let mut items = packages::scan(&fleet)?;
    items.extend(tools::scan(&fleet)?);
    items.extend(files::scan()?);
    items.extend(home::scan(&fleet, &managed)?);
    items.extend(etc::scan(&fleet)?);
    items.extend(services::scan(&fleet)?);
    items.extend(account::scan(&fleet)?);
    items.extend(settings::scan(&managed)?);

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

pub fn run() -> Result<()> {
    let report = collect()?;
    for kind in Kind::ALL {
        let group: Vec<&Item> = report
            .shown
            .iter()
            .filter(|item| item.kind == kind)
            .collect();
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
            let secret = if item.secret { "  [secret]" } else { "" };
            println!("  {:width$}  {}{secret}", item.subject, item.detail);
        }
        println!();
    }
    println!(
        "{} differences, {} ignored by {}",
        report.shown.len(),
        report.ignored,
        crate::fleet::tilde(&report.fleet.ignore_path())
    );
    Ok(())
}

/// A guess, so the report can warn before anything gets committed in clear.
fn looks_secret(mut item: Item, path: &Path) -> Item {
    item.secret = guess_secret(path);
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
