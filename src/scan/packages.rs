use super::{Item, Kind};
use crate::{edit::Add, fleet::Fleet, pacman::Db, preview::Preview};
use anyhow::Result;
use std::collections::BTreeSet;

pub fn scan(fleet: &Fleet, db: &Db) -> Result<Vec<Item>> {
    let mut listed = BTreeSet::new();
    for name in fleet
        .system
        .packages
        .iter()
        .chain(&fleet.system.built_packages)
    {
        listed.extend(fleet.package_list(name)?);
    }
    let installed: BTreeSet<&String> = db.packages.iter().map(|p| &p.name).collect();
    let mut items: Vec<Item> = db
        .packages
        .iter()
        .filter(|p| p.explicit && !listed.contains(&p.name))
        .map(|p| {
            Item::new(Kind::Package, &p.name, "installed, not listed")
                .with(Add::ListPackage(p.name.clone()))
                .showing(Preview::Text(p.description.clone()))
        })
        .collect();
    items.extend(
        listed
            .iter()
            .filter(|name| !installed.contains(name))
            .map(|name| {
                Item::new(Kind::Package, name, "listed, not installed")
                    .with(Add::UnlistPackage(name.clone()))
            }),
    );
    Ok(items)
}
