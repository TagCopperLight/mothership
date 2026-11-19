use crate::item::{Change, Item};
use crate::kind::Kind;
use crate::{edit::Add, fleet::Fleet, package::Db, revert::Machine};
use anyhow::Result;
use std::collections::BTreeSet;

pub fn scan(fleet: &Fleet, db: &Db) -> Result<Vec<Item>> {
    let installed_here = fleet.packages();
    let listed = fleet.listed_packages();
    let covered: BTreeSet<&str> = listed
        .iter()
        .filter_map(|name| db.resolve(name))
        .map(|package| package.name.as_str())
        .collect();
    let mut items: Vec<Item> = db
        .packages
        .iter()
        .filter(|p| p.explicit && !covered.contains(p.name.as_str()))
        .map(|p| {
            Item::new(Kind::Package, Change::Added, &p.name, "installed")
                .noting(&p.description)
                .with(Add::ListPackage(p.name.clone()))
                .reverting(Machine::Uninstall(p.name.clone()))
        })
        .collect();
    items.extend(
        listed
            .iter()
            .filter(|name| db.resolve(name).is_none())
            .map(|name| {
                let item = |detail| {
                    Item::new(Kind::Package, Change::Removed, name, detail)
                        .with(Add::UnlistPackage(name.clone()))
                };
                if installed_here.contains(name) {
                    item("uninstalled").reverting(Machine::Install(name.clone()))
                } else {
                    item("not installed").noting("built elsewhere, nothing installs it")
                }
            }),
    );
    Ok(items)
}
