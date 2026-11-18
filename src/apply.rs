use crate::{
    fleet::Fleet,
    pacman::Db,
    prompt::Prompt,
    record::{self, Record},
    revert::{self, Owned, Revert},
    scan::{self, Checks, Item, Secret},
};
use anyhow::Result;
use std::path::Path;

/// A new user, or a user's chezmoi set up, brings checks the scan couldn't make before, so apply
/// goes round again until a round brings none.
pub fn run() -> Result<()> {
    let mut prompt = Prompt::new();
    while round(&mut prompt)? {}
    Ok(())
}

/// Whether to go round again.
fn round(prompt: &mut Prompt) -> Result<bool> {
    let fleet = Fleet::load()?;
    let db = Db::load()?;
    let record_path = Path::new(record::PATH);
    let record = Record::load(record_path)?;
    let mut items = scan::checks(&fleet, &db, Checks::Apply)?;
    items.sort_by_key(|item| (item.member, item.kind));

    let (mut todo, mut edited, mut locked, mut left) = (Vec::new(), Vec::new(), Vec::new(), 0);
    for item in &items {
        let user = item.member.map(|i| fleet.members[i].name.as_str());
        match &item.revert {
            Some(revert) if record.fleet_changed(revert, user, &db)? => todo.push(item),
            _ if scan::ignored(&fleet, item) => {}
            _ if item.secret == Secret::Locked => locked.push(item),
            Some(Revert::WriteEtc(_) | Revert::ApplyFile { .. }) => edited.push(item),
            _ => left += 1,
        }
    }

    scan::print(&fleet, &todo, |item| {
        item.revert.as_ref().expect("filtered above").describe()
    });
    for (items, why) in [
        (&edited, "they were edited on this machine"),
        (&locked, "this machine's keys can't open them"),
    ] {
        if !items.is_empty() {
            println!("Left alone, since {why}:");
            for item in items {
                println!("  {}", shown(&fleet, item));
            }
            println!();
        }
    }
    if left > 0 {
        println!("{left} other differences wait for `mothership review`.");
    }
    if todo.is_empty() {
        println!("Nothing to apply.");
    } else if prompt.confirm("Apply")? {
        let reverts: Vec<Owned> = todo
            .iter()
            .map(|item| (item.member, item.revert.as_ref().expect("filtered above")))
            .collect();
        revert::run(&fleet, &reverts)?;
    } else {
        return Ok(false);
    }
    Record::after(&fleet, &record, Path::new("/etc"))?.save(record_path)?;
    Ok(todo
        .iter()
        .any(|item| matches!(item.revert, Some(Revert::CreateUser | Revert::InitChezmoi))))
}

/// A user's item goes by the user's name.
fn shown(fleet: &Fleet, item: &Item) -> String {
    match item.member {
        None => item.subject.clone(),
        Some(i) => format!("{}: {}", fleet.members[i].name, item.subject),
    }
}
