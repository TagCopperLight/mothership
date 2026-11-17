use super::{Item, Kind};
use crate::{cmd, edit::Add, fleet::Fleet};
use anyhow::Result;
use std::collections::BTreeSet;

pub fn scan(fleet: &Fleet) -> Result<Vec<Item>> {
    let mut listed = BTreeSet::new();
    for name in fleet
        .system
        .packages
        .iter()
        .chain(&fleet.system.built_packages)
    {
        listed.extend(fleet.package_list(name)?);
    }
    let explicit = pacman(&["-Qqe"])?;
    let installed = pacman(&["-Qq"])?;
    let mut items: Vec<Item> = explicit
        .difference(&listed)
        .map(|name| {
            Item::new(Kind::Package, name, "installed, not listed")
                .with(Add::ListPackage(name.clone()))
        })
        .collect();
    items.extend(listed.difference(&installed).map(|name| {
        Item::new(Kind::Package, name, "listed, not installed")
            .with(Add::UnlistPackage(name.clone()))
    }));
    Ok(items)
}

fn pacman(args: &[&str]) -> Result<BTreeSet<String>> {
    Ok(cmd::stdout("pacman", args)?
        .lines()
        .map(str::to_owned)
        .collect())
}
