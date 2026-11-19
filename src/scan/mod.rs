mod account;
mod etc;
pub mod files;
mod home;
mod packages;
mod print;
mod secret;
mod services;
mod settings;
pub mod tools;

pub use print::{Columns, heading, line, print_groups, print_section};

use crate::{
    chezmoi,
    fleet::{self, Fleet, Member},
    item::{Change, Item, Section},
    kind::Kind,
    later::{self, Later},
    package::Db,
    parallel::join,
    record::Record,
    revert,
    style::{Paint, Style},
    walk,
};
use anyhow::Result;
use print::{owner, summary};
use secret::finish;
use std::{collections::HashSet, fs, path::Path, thread};

pub fn ignored(fleet: &Fleet, item: &Item) -> bool {
    item.section != Section::Fleet
        && match item.member {
            None => fleet.ignore.matches(item.kind, &item.subject),
            Some(i) => fleet.members[i].ignore.matches(item.kind, &item.subject),
        }
}

pub struct Report {
    pub fleet: Fleet,
    pub shown: Vec<Item>,
    pub ignored: usize,
}

pub fn collect() -> Result<Report> {
    let (fleet, db) = thread::scope(|s| {
        let fleet = s.spawn(Fleet::load);
        let db = s.spawn(Db::load);
        Ok::<_, anyhow::Error>((join(fleet)?, join(db)?))
    })?;
    let record = Record::load()?;
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
    let ships = Ships::new(fleet, db);
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
            .reverting(revert::User::Create),
        ),
        Some(_) => thread::scope(|s| {
            let parts = [
                (Kind::Account, s.spawn(|| account::scan(member, ships))),
                (
                    Kind::UserService,
                    s.spawn(|| services::scan_user(member, ships)),
                ),
                (Kind::Tool, s.spawn(|| tools::scan(member))),
                (Kind::File, s.spawn(|| tracked(fleet, member, which))),
            ];
            for (kind, part) in parts {
                match join(part) {
                    Ok(found) => items.extend(found),
                    Err(e) => items.push(
                        Item::new(kind, Change::Unreadable, kind.area().title(), "not scanned")
                            .noting(format!("{e:#}")),
                    ),
                }
            }
        }),
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
                .noting("its config isn't the one mothership writes")
                .reverting(revert::User::InitChezmoi),
        ]);
    }
    let chezmoi = files::Chezmoi::load(member)?;
    let mut items = files::scan(fleet, member, &chezmoi)?;
    if which == Checks::All {
        let managed = &chezmoi.managed;
        thread::scope(|s| {
            let home = s.spawn(|| home::scan(member, managed));
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
    files: HashSet<&'a str>,
}

impl<'a> Ships<'a> {
    pub fn new(fleet: &'a Fleet, db: &'a Db) -> Self {
        let listed = db.with_depends(&fleet.listed_packages());
        let files = db
            .packages
            .iter()
            .filter(|package| listed.contains(&package.name))
            .flat_map(|package| &package.files)
            .map(String::as_str)
            .collect();
        Ships { fleet, db, files }
    }

    pub fn path(&self, path: &Path, member: Option<&Member>) -> Result<bool> {
        let resolved = match (path.parent(), path.file_name()) {
            (Some(dir), Some(name)) => match walk::if_found(fs::canonicalize(dir))? {
                Some(dir) => dir.join(name),
                None => return Ok(false),
            },
            _ => return Ok(false),
        };
        if let Ok(rel) = resolved.strip_prefix(fleet::ETC)
            && self.fleet.etc.contains(rel)
        {
            return Ok(true);
        }
        if let Some(user) = member.and_then(|m| m.user.as_ref())
            && resolved.starts_with(&user.home)
        {
            return chezmoi::manages(user, &resolved);
        }
        Ok(self.files.contains(resolved.to_string_lossy().as_ref()))
    }
}

pub fn run() -> Result<()> {
    let report = collect()?;
    let fleet = &report.fleet;
    let style = Style::stdout();
    let later = Later::load()?;
    let now = later::now()?;
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

#[cfg(test)]
#[path = "../../tests/scan.rs"]
mod tests;
