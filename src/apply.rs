use crate::{
    file,
    fleet::Fleet,
    item::{Change, Secret, Section},
    package::Db,
    prompt::Prompt,
    record::Record,
    revert::{self, Revert, Service, User},
    scan::{self, Checks},
    style::Style,
};
use anyhow::Result;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ServiceManager {
    Running,
    Missing,
}

pub fn run() -> Result<()> {
    let mut prompt = Prompt::new();
    while round(&mut || prompt.confirm("Apply"), ServiceManager::Running)? {}
    Ok(())
}

pub fn unasked(service_manager: ServiceManager) -> Result<()> {
    while round(&mut || Ok(true), service_manager)? {}
    Ok(())
}

fn round(
    approve: &mut dyn FnMut() -> Result<bool>,
    service_manager: ServiceManager,
) -> Result<bool> {
    let fleet = Fleet::load()?;
    let db = Db::load()?;
    let record = Record::load()?;
    let mut items = scan::checks(&fleet, &db, &record, Checks::Apply)?;
    scan::sort(&fleet, &mut items);

    let (mut todo, mut reverts, mut edited, mut locked, mut unread, mut left) = (
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        0,
    );
    let mut skipped: Vec<(usize, &Service)> = Vec::new();
    for item in &items {
        match (&item.revert, item.member) {
            (Some(Revert::User(User::Service(service))), Some(i))
                if item.section == Section::Fleet && service_manager == ServiceManager::Missing =>
            {
                skipped.push((i, service))
            }
            (Some(revert), member) if item.section == Section::Fleet => {
                todo.push(item);
                reverts.push((member, revert));
            }
            _ if scan::ignored(&fleet, item) => {}
            _ if item.secret == Secret::Locked => locked.push(item),
            _ if item.change == Change::Unreadable => unread.push(item),
            (Some(revert), _) if revert.writes_fleet_version() => edited.push(item),
            _ => left += 1,
        }
    }

    let style = Style::stdout();
    scan::print_section(&fleet, Section::Fleet, &todo, style, &|_| None);
    if !skipped.is_empty() {
        println!(
            "{} left for the machine's first start, which runs a service manager",
            skipped.len()
        );
    }
    for (items, why) in [
        (&edited, "they were edited on this machine"),
        (&locked, "this machine's key can't open them"),
        (&unread, "mothership couldn't read them"),
    ] {
        if !items.is_empty() {
            println!("Left alone, since {why}:");
            scan::print_groups(&fleet, items, style, &|_| None);
        }
    }
    let review = left + edited.len() + locked.len() + unread.len();
    if review > 0 {
        println!(
            "{review} changed on this machine, for {}",
            style.command("review")
        );
    }
    if todo.is_empty() {
        println!("Nothing to apply.");
    } else if approve()? {
        revert::run(&fleet, &reverts)?;
    } else {
        return Ok(false);
    }
    Record::after(&fleet, &record, Path::new(file::ETC), &skipped)?.save()?;
    Ok(todo.iter().any(|item| {
        matches!(
            item.revert,
            Some(Revert::User(User::Create | User::InitChezmoi))
        )
    }))
}
