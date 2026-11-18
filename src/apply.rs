use crate::{
    fleet::Fleet,
    pacman::Db,
    prompt::Prompt,
    record::{self, Record},
    revert::{self, Revert},
    scan::{self, Secret},
    user,
};
use anyhow::Result;
use std::path::Path;

pub fn run() -> Result<()> {
    let fleet = Fleet::load()?;
    let db = Db::load()?;
    let record_path = Path::new(record::PATH);
    let record = Record::load(record_path)?;
    let mut items = scan::system(&fleet, &db)?;
    items.extend(scan::tools::scan(&fleet)?);
    items.extend(scan::files::scan(&fleet, &scan::managed()?)?);

    let (mut todo, mut edited, mut locked, mut left) = (Vec::new(), Vec::new(), Vec::new(), 0);
    for item in &items {
        match &item.revert {
            Some(revert) if record.fleet_changed(revert, &db)? => todo.push(item),
            _ if fleet.ignore.matches(item) => {}
            _ if item.secret == Secret::Locked => locked.push(item),
            Some(Revert::WriteEtc(_) | Revert::ApplyFile { .. }) => edited.push(item),
            _ => left += 1,
        }
    }

    scan::print(&todo, |item| {
        item.revert.as_ref().expect("filtered above").describe()
    });
    for (items, why) in [
        (&edited, "they were edited on this machine"),
        (&locked, "this machine's key can't open them"),
    ] {
        if !items.is_empty() {
            println!("Left alone, since {why}:");
            for item in items {
                println!("  {}", item.subject);
            }
            println!();
        }
    }
    if left > 0 {
        println!("{left} other differences wait for `mothership review`.");
    }
    if todo.is_empty() {
        println!("Nothing to apply.");
    } else if !user::is_root() {
        println!("Run it with sudo to apply. Without root, unreadable /etc files are left out.");
        return Ok(());
    } else if Prompt::new().confirm("Apply")? {
        let reverts: Vec<&Revert> = todo.iter().filter_map(|i| i.revert.as_ref()).collect();
        revert::run(&fleet, &reverts)?;
    } else {
        return Ok(());
    }
    if user::is_root() {
        Record::after(&fleet, &record, Path::new("/etc"))?.save(record_path)?;
    }
    Ok(())
}
