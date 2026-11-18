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
    later::{self, Later},
    package::{Db, Distro},
    preview::Preview,
    record::{self, Record},
    revert::Revert,
    style::{self, Paint, Style},
    walk,
};
use anyhow::Result;
use std::{
    collections::BTreeSet,
    fs,
    io::ErrorKind,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    thread::{self, ScopedJoinHandle},
};

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

    pub fn level(self) -> Level {
        match self {
            Kind::Package | Kind::Etc | Kind::Service => Level::Machine,
            _ => Level::User,
        }
    }

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

    pub fn parse(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|kind| kind.name() == name)
    }

    fn area(self) -> Area {
        match self {
            Kind::Package => Area::Packages,
            Kind::Tool => Area::Tools,
            Kind::File | Kind::Home | Kind::Etc => Area::Files,
            Kind::Service => Area::Services,
            Kind::UserService => Area::UserServices,
            Kind::Account => Area::Account,
            Kind::Dconf | Kind::Pref => Area::Settings,
        }
    }
}

/// How scan groups what it finds: the way I think of a machine, not the way it checks it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Area {
    Packages,
    Tools,
    Files,
    Services,
    UserServices,
    Account,
    Settings,
}

impl Area {
    fn title(self) -> &'static str {
        match self {
            Area::Packages => "Packages",
            Area::Tools => "Tools",
            Area::Files => "Files",
            Area::Services => "Services",
            Area::UserServices => "User services",
            Area::Account => "Account",
            Area::Settings => "Settings",
        }
    }
}

/// What differs, seen from the machine: Added means the machine has it and the fleet doesn't.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Change {
    /// A file that looks secret, which the fleet holds in clear.
    InClear,
    Unreadable,
    Added,
    Edited,
    Removed,
}

/// Who moved: the fleet, which apply follows, or the machine, which waits for review.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Section {
    Attention,
    Machine,
    Fleet,
}

impl Section {
    pub const ALL: [Section; 3] = [Section::Attention, Section::Machine, Section::Fleet];

    pub fn title(self) -> &'static str {
        match self {
            Section::Attention => "Needs attention",
            Section::Machine => "Changed on this machine",
            Section::Fleet => "Coming from the fleet",
        }
    }

    fn paint(self) -> Paint {
        match self {
            Section::Attention => Paint::Red,
            Section::Machine => Paint::Yellow,
            Section::Fleet => Paint::Cyan,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Secret {
    No,
    Guessed,
    Fleet,
    Locked,
}

pub struct Item {
    pub kind: Kind,
    pub change: Change,
    pub section: Section,
    pub member: Option<usize>,
    pub subject: String,
    /// What happened on the machine, in a word or two: "installed", "edited", "joined".
    pub detail: String,
    /// Facts that help decide, such as a package's description or a folder's size.
    pub note: String,
    pub secret: Secret,
    pub add: Option<Add>,
    pub no_add: Option<&'static str>,
    pub revert: Option<Revert>,
    pub preview: Preview,
    /// Guessed only for the items shown, since searching a big ignored folder takes seconds.
    pub guess: Option<PathBuf>,
}

impl Item {
    pub fn new(
        kind: Kind,
        change: Change,
        subject: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        let section = match change {
            Change::InClear | Change::Unreadable => Section::Attention,
            Change::Added | Change::Edited | Change::Removed => Section::Machine,
        };
        Item {
            kind,
            change,
            section,
            member: None,
            subject: subject.into(),
            detail: detail.into(),
            note: String::new(),
            secret: Secret::No,
            add: None,
            no_add: None,
            revert: None,
            preview: Preview::None,
            guess: None,
        }
    }

    fn locked(kind: Kind, subject: impl Into<String>) -> Self {
        let mut item = Item::new(kind, Change::Unreadable, subject, "locked")
            .noting("this machine's key isn't on its list");
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

    fn noting(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }

    fn with(mut self, add: Add) -> Self {
        self.add = Some(add);
        self
    }

    fn guessing(mut self, path: &Path) -> Self {
        self.guess = Some(path.to_path_buf());
        self
    }

    fn because(mut self, no_add: &'static str) -> Self {
        self.add = None;
        self.no_add = Some(no_add);
        self
    }

    /// The sign shows what happens to the machine: what it gained, or what apply brings it.
    fn sign(&self) -> (&'static str, Paint) {
        let change = match (self.section, self.change) {
            (Section::Fleet, Change::Added) => Change::Removed,
            (Section::Fleet, Change::Removed) => Change::Added,
            (_, change) => change,
        };
        match change {
            Change::InClear => ("!", Paint::Red),
            Change::Unreadable => ("?", Paint::Dim),
            Change::Added => ("+", Paint::Green),
            Change::Edited => ("~", Paint::Yellow),
            Change::Removed => ("-", Paint::Red),
        }
    }

    /// On the fleet's side, what apply does to follow it.
    fn verb(&self) -> String {
        match (&self.revert, self.section) {
            // Writing the fleet's version of a file can create it, change it or delete it.
            (Some(Revert::ApplyFile { .. } | Revert::WriteEtc(_)), Section::Fleet) => {
                match self.change {
                    Change::Added => "delete",
                    Change::Removed => "create",
                    _ => "update",
                }
                .into()
            }
            (Some(revert), Section::Fleet) => revert.describe(),
            _ => self.detail.clone(),
        }
    }
}

pub fn ignore_line(item: &Item) -> String {
    format!("{} {}", item.kind.name(), globset::escape(&item.subject))
}

pub fn key(fleet: &Fleet, item: &Item) -> String {
    match item.member {
        None => ignore_line(item),
        Some(i) => format!("{}: {}", fleet.members[i].name, ignore_line(item)),
    }
}

/// The owner shows only on a type with several users: with one, `~` and the area say it.
pub fn heading(fleet: &Fleet, item: &Item) -> String {
    let area = item.kind.area().title();
    match owner(fleet, item) {
        Some(i) => format!("{area} · {}", fleet.members[i].name),
        None => area.to_owned(),
    }
}

fn owner(fleet: &Fleet, item: &Item) -> Option<usize> {
    item.member.filter(|_| fleet.members.len() > 1)
}

/// The fleet's changes come whatever the ignore lists say, as apply follows them.
pub fn ignored(fleet: &Fleet, item: &Item) -> bool {
    item.section != Section::Fleet
        && match item.member {
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
    Distro::here()?;
    let (fleet, db) = thread::scope(|s| {
        let fleet = s.spawn(Fleet::load);
        let db = s.spawn(Db::load);
        Ok::<_, anyhow::Error>((join(fleet)?, join(db)?))
    })?;
    let record = Record::load(Path::new(record::PATH))?;
    let items = checks(&fleet, &db, &record, Checks::All)?;
    let (ignored, shown): (Vec<Item>, Vec<Item>) =
        items.into_iter().partition(|item| ignored(&fleet, item));
    let mut shown: Vec<Item> = shown.into_iter().map(|item| finish(&fleet, item)).collect();
    sort(&fleet, &mut shown);
    Ok(Report {
        fleet,
        shown,
        ignored: ignored.len(),
    })
}

pub fn sort(fleet: &Fleet, items: &mut [Item]) {
    items.sort_by(|a, b| {
        let key = |item: &Item| {
            (
                item.section,
                item.kind.area(),
                owner(fleet, item),
                item.change,
            )
        };
        key(a).cmp(&key(b)).then_with(|| a.subject.cmp(&b.subject))
    });
}

#[derive(Clone, Copy, PartialEq)]
pub enum Checks {
    All,
    Apply,
}

pub fn checks(fleet: &Fleet, db: &Db, record: &Record, which: Checks) -> Result<Vec<Item>> {
    let ships = Ships::new(fleet, db)?;
    let mut items = thread::scope(|s| {
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
        Ok::<_, anyhow::Error>(items.into_iter().flatten().collect::<Vec<_>>())
    })?;
    for item in &mut items {
        let user = item.member.map(|i| fleet.members[i].name.as_str());
        if item.section == Section::Machine
            && let Some(revert) = &item.revert
            && record.fleet_changed(revert, user, db)?
        {
            item.section = Section::Fleet;
        }
    }
    Ok(items)
}

fn member(fleet: &Fleet, ships: &Ships, i: usize, which: Checks) -> Result<Vec<Item>> {
    let member = &fleet.members[i];
    let mut items = Vec::new();
    match &member.user {
        None => items.push(
            Item::new(
                Kind::Account,
                Change::Removed,
                format!("user {}", member.name),
                "missing",
            )
            .reverting(Revert::CreateUser),
        ),
        Some(_) => {
            let parts = thread::scope(|s| {
                let handles = [
                    s.spawn(|| account::scan(member, ships)),
                    s.spawn(|| services::scan_user(member, ships)),
                    s.spawn(|| tools::scan(member)),
                    s.spawn(|| tracked(fleet, member, which)),
                ];
                handles.into_iter().map(join).collect::<Result<Vec<_>>>()
            })?;
            items.extend(parts.into_iter().flatten());
        }
    }
    for item in &mut items {
        item.member = Some(i);
    }
    Ok(items)
}

fn tracked(fleet: &Fleet, member: &Member, which: Checks) -> Result<Vec<Item>> {
    if !member.chezmoi {
        return Ok(vec![
            Item::new(Kind::File, Change::Removed, "chezmoi", "not set up")
                .noting("its config doesn't read the fleet")
                .reverting(Revert::InitChezmoi),
        ]);
    }
    let chezmoi = files::Chezmoi::load(member)?;
    let mut items = files::scan(member, &chezmoi)?;
    if which == Checks::All {
        let managed = &chezmoi.managed;
        thread::scope(|s| {
            let home = s.spawn(|| home::scan(fleet, member, managed));
            items.extend(settings::scan(member, managed)?);
            items.extend(join(home)?);
            Ok::<_, anyhow::Error>(())
        })?;
    }
    Ok(items)
}

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
    let fleet = &report.fleet;
    let style = Style::stdout();
    let later = Later::load()?;
    let now = later::now()?;
    // Put off applies to my changes: the fleet's come with the next apply.
    let waiting = |item: &Item| {
        (item.section != Section::Fleet)
            .then(|| later.days_left(fleet, item, now))
            .flatten()
    };
    for section in Section::ALL {
        let items: Vec<&Item> = report
            .shown
            .iter()
            .filter(|item| item.section == section)
            .collect();
        print_section(fleet, section, &items, style, &waiting);
    }
    summary(&report, style, &waiting);
    let unsaved = fleet.unsaved()?;
    if !unsaved.is_empty() {
        let git = format!("git -C {} status", fleet.dir.display());
        println!(
            "{}  {}",
            style.paint(
                format!("The fleet has {}", unsaved.join(" and ")),
                &[Paint::Yellow]
            ),
            style.paint(git, &[Paint::Cyan])
        );
    }
    let ignored = format!("{} ignored", report.ignored);
    println!("{}", style.paint(ignored, &[Paint::Dim]));
    Ok(())
}

fn summary(report: &Report, style: Style, waiting: &dyn Fn(&Item) -> Option<u64>) {
    let count = |section| {
        report
            .shown
            .iter()
            .filter(|item| item.section == section)
            .count()
    };
    let put_off = report
        .shown
        .iter()
        .filter(|item| item.section != Section::Fleet && waiting(item).is_some())
        .count();
    let review = count(Section::Attention) + count(Section::Machine) - put_off;
    let apply = count(Section::Fleet);
    let bold = |n: usize| style.paint(n, &[Paint::Bold]);
    if review + apply + put_off == 0 {
        println!(
            "{} Nothing differs from the {} type",
            style.paint("✓", &[Paint::Green, Paint::Bold]),
            report.fleet.type_name
        );
    }
    if review > 0 {
        println!("{} to review     {}", bold(review), style.command("review"));
    }
    if apply > 0 {
        println!("{} to apply      {}", bold(apply), style.command("apply"));
    }
    if put_off > 0 {
        let later = format!("{put_off} put off for later");
        println!("{}", style.paint(later, &[Paint::Blue]));
    }
}

pub fn print_section(
    fleet: &Fleet,
    section: Section,
    items: &[&Item],
    style: Style,
    waiting: &dyn Fn(&Item) -> Option<u64>,
) {
    if items.is_empty() {
        return;
    }
    let title = format!("{} · {}", section.title(), items.len());
    println!("{}", style.paint(title, &[Paint::Bold, section.paint()]));
    print_groups(fleet, items, style, waiting);
}

/// Items under their area's heading, in the order given.
pub fn print_groups(
    fleet: &Fleet,
    items: &[&Item],
    style: Style,
    waiting: &dyn Fn(&Item) -> Option<u64>,
) {
    let columns = Columns::of(items);
    for group in items.chunk_by(|a, b| heading(fleet, a) == heading(fleet, b)) {
        println!(
            "  {}",
            style.paint(heading(fleet, group[0]), &[Paint::Bold])
        );
        for item in group {
            println!("{}", line(item, style, columns, waiting(item)));
        }
    }
    println!();
}

/// Widths that line up the subjects and verbs of a section. A long subject pushes its own
/// line's verb out instead of widening every line.
#[derive(Clone, Copy)]
pub struct Columns {
    subject: usize,
    verb: usize,
}

impl Columns {
    const SUBJECT: usize = 40;

    pub fn of(items: &[&Item]) -> Self {
        let widest = |text: &dyn Fn(&Item) -> usize| items.iter().map(|i| text(i)).max();
        Columns {
            subject: widest(&|i| style::visible(&i.subject))
                .unwrap_or(0)
                .min(Columns::SUBJECT),
            verb: widest(&|i| style::visible(&i.verb())).unwrap_or(0),
        }
    }
}

/// One item on one line, the same in scan, review and apply.
pub fn line(item: &Item, style: Style, columns: Columns, waiting: Option<u64>) -> String {
    let (sign, paint) = item.sign();
    let verb_paint = match item.section {
        Section::Fleet => Paint::Cyan,
        _ => paint,
    };
    let mut extras = Vec::new();
    // A locked item and one in clear say it in their own words.
    if matches!(item.secret, Secret::Guessed | Secret::Fleet) && item.change != Change::InClear {
        extras.push(style.paint("secret", &[Paint::Magenta, Paint::Bold]));
    }
    if let Some(days) = waiting {
        let left = format!("later, back in {}", style::count(days as usize, "day"));
        extras.push(style.paint(left, &[Paint::Blue]));
    }
    if !item.note.is_empty() {
        extras.push(style.paint(&item.note, &[Paint::Dim]));
    }
    let subject = if waiting.is_some() {
        style.paint(&item.subject, &[Paint::Dim])
    } else {
        item.subject.clone()
    };
    let text = format!(
        "    {} {}  {}  {}",
        style.paint(sign, &[paint, Paint::Bold]),
        style::pad(&subject, columns.subject),
        style::pad(&style.paint(item.verb(), &[verb_paint]), columns.verb),
        extras.join("  "),
    );
    let text = text.trim_end();
    if style.terminal {
        style::clip(text, style::width())
    } else {
        text.to_owned()
    }
}

/// Adds what only the shown items need, since ignored ones can be huge folders.
fn finish(fleet: &Fleet, mut item: Item) -> Item {
    let Some(path) = item.guess.take() else {
        return item;
    };
    if item.change == Change::Added && item.note.is_empty() {
        item.note = size(&path);
    }
    let not_secret = match item.member {
        None => &fleet.not_secret,
        Some(i) => &fleet.members[i].not_secret,
    };
    looks_secret(not_secret, item, &path)
}

/// A plain file needs no note: its preview shows it.
fn size(path: &Path) -> String {
    let described = match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_symlink() => fs::read_link(path)
            .map(|target| format!("→ {}", target.display()))
            .map_err(anyhow::Error::from),
        Ok(meta) if meta.is_dir() => {
            walk::files(path).map(|files| style::count(files.len(), "file"))
        }
        Ok(_) => return String::new(),
        Err(e) => Err(e.into()),
    };
    described.unwrap_or_else(|e| format!("can't read it: {e:#}"))
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
