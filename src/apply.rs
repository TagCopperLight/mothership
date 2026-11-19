use crate::{
    fleet::Fleet,
    package::Db,
    prompt::Prompt,
    record::{self, Record},
    revert::{self, Owned, Revert},
    scan::{self, Change, Checks, Secret, Section},
    style::Style,
};
use anyhow::Result;
use std::path::Path;

pub fn run() -> Result<()> {
    let mut prompt = Prompt::new();
    while round(&mut || prompt.confirm("Apply"), true)? {}
    Ok(())
}

pub fn unasked(service_manager: bool) -> Result<()> {
    while round(&mut || Ok(true), service_manager)? {}
    Ok(())
}

fn round(approve: &mut dyn FnMut() -> Result<bool>, service_manager: bool) -> Result<bool> {
    let fleet = Fleet::load()?;
    let db = Db::load()?;
    let record_path = Path::new(record::PATH);
    let record = Record::load(record_path)?;
    let mut items = scan::checks(&fleet, &db, &record, Checks::Apply)?;
    scan::sort(&fleet, &mut items);

    let (mut todo, mut edited, mut locked, mut unread, mut left) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), 0);
    let mut skipped: Vec<Owned> = Vec::new();
    for item in &items {
        match &item.revert {
            Some(revert)
                if item.section == Section::Fleet
                    && !service_manager
                    && revert.needs_service_manager() =>
            {
                skipped.push((item.member, revert))
            }
            _ if item.section == Section::Fleet => todo.push(item),
            _ if scan::ignored(&fleet, item) => {}
            _ if item.secret == Secret::Locked => locked.push(item),
            _ if item.change == Change::Unreadable => unread.push(item),
            Some(Revert::WriteEtc(_) | Revert::ApplyFile { .. }) => edited.push(item),
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
        let reverts: Vec<Owned> = todo
            .iter()
            .map(|item| (item.member, item.revert.as_ref().expect("filtered above")))
            .collect();
        revert::run(&fleet, &reverts)?;
    } else {
        return Ok(false);
    }
    Record::after(&fleet, &record, Path::new("/etc"), &skipped)?.save(record_path)?;
    Ok(todo
        .iter()
        .any(|item| matches!(item.revert, Some(Revert::CreateUser | Revert::InitChezmoi))))
}
