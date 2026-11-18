use super::{Item, Kind};
use crate::{edit::Add, fleet::Fleet, pacman::Db, preview::Preview, revert::Revert};
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
            Item::new(Kind::Package, &p.name, "installed, not listed")
                .with(Add::ListPackage(p.name.clone()))
                .reverting(Revert::Uninstall(p.name.clone()))
                .showing(Preview::Text(p.description.clone()))
        })
        .collect();
    items.extend(
        listed
            .iter()
            .filter(|name| !installed.contains(name))
            .map(|name| {
                let item = Item::new(Kind::Package, name, "listed, not installed")
                    .with(Add::UnlistPackage(name.clone()));
                // Other repos build these, so there's nothing to install them from.
                if installed_here.contains(name) {
                    item.reverting(Revert::Install(name.clone()))
                } else {
                    item
                }
            }),
    );
    Ok(items)
}
