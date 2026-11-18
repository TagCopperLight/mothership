use super::{Change, Item, Kind};
use crate::{edit::Add, fleet::Fleet, pacman::Db, revert::Revert};
use anyhow::Result;
use std::collections::BTreeSet;

pub fn scan(fleet: &Fleet, db: &Db) -> Result<Vec<Item>> {
    let installed_here = fleet.packages();
    let mut listed = installed_here.clone();
    listed.extend(fleet.system.built_packages.iter().cloned());
    let installed: BTreeSet<&String> = db.packages.iter().map(|p| &p.name).collect();
    let mut items: Vec<Item> = db
        .packages
        .iter()
        .filter(|p| p.explicit && !listed.contains(&p.name))
        .map(|p| {
            Item::new(Kind::Package, Change::Added, &p.name, "installed")
                .noting(&p.description)
                .with(Add::ListPackage(p.name.clone()))
                .reverting(Revert::Uninstall(p.name.clone()))
        })
        .collect();
    items.extend(
        listed
            .iter()
            .filter(|name| !installed.contains(name))
            .map(|name| {
                let item = |detail| {
                    Item::new(Kind::Package, Change::Removed, name, detail)
                        .with(Add::UnlistPackage(name.clone()))
                };
                // Other repos build these, so there's nothing to install them from.
                if installed_here.contains(name) {
                    item("uninstalled").reverting(Revert::Install(name.clone()))
                } else {
                    item("not installed").noting("built elsewhere, nothing installs it")
                }
            }),
    );
    Ok(items)
}
