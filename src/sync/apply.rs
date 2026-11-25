use crate::{
    fleet::{Fleet, Unsaved},
    package::Db,
    services,
    sync::item::{Change, Secret, Section},
    sync::record::Record,
    sync::revert::{self, Machine, Revert, User},
    sync::scan::{self, Checks},
    trust,
    trust::update::{self, Outcome},
    usage,
    util::file,
    util::prompt::Prompt,
    util::style::Style,
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
    let Some(fleet) = follow(
        &mut |question| prompt.confirm(question),
        ServiceManager::Running,
    )?
    else {
        return Ok(());
    };
    scan::print_unsaved(&fleet, Unsaved::Uncommitted, Style::stdout())?;
    services_settled()
}

pub fn unasked(service_manager: ServiceManager) -> Result<()> {
    follow(&mut |_| Ok(true), service_manager).map(drop)
}

fn follow(
    ask: &mut dyn FnMut(&str) -> Result<bool>,
    service_manager: ServiceManager,
) -> Result<Option<Fleet>> {
    if let Outcome::Declined = update::follow_fleet(&mut || ask("Update"))? {
        println!("Apply stops: the fleet's changes may need that version.");
        return Ok(None);
    }
    while round(&mut || ask("Apply"), service_manager)? {}
    let fleet = Fleet::load()?;
    usage::keep_completion()?;
    trust::create_keys(&fleet)?;
    Ok(Some(fleet))
}

fn services_settled() -> Result<()> {
    let problems = services::problems()?;
    if !problems.is_empty() {
        anyhow::bail!("these services need attention:\n{}", problems.join("\n"));
    }
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
    let mut skipped: Vec<(usize, &User)> = Vec::new();
    for item in &items {
        match (&item.revert, item.member) {
            (Some(Revert::User(change)), Some(i))
                if item.section == Section::Fleet
                    && service_manager == ServiceManager::Missing
                    && change.needs_service_manager() =>
            {
                skipped.push((i, change))
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
    scan::print_section(&fleet, Section::Fleet, &todo, style);
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
            scan::print_groups(&fleet, items, style);
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
            Some(Revert::Machine(Machine::CreateUser { .. }) | Revert::User(User::InitChezmoi))
        )
    }))
}
