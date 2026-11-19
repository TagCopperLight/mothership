use crate::item::{Change, Item};
use crate::kind::Kind;
use crate::{edit::Add, fleet::Fleet, package::Db, revert::Machine};
use anyhow::{Result, bail};
use std::collections::BTreeSet;

fn refuse_provided(db: &Db, listed: &BTreeSet<String>) -> Result<()> {
    for name in listed {
        if let Some(package) = db.resolve(name)
            && package.name != *name
        {
            bail!(
                "{name} isn't a package, {} provides it: list {} instead",
                package.name,
                package.name
            );
        }
    }
    Ok(())
}

fn refuse_unknown(fleet: &Fleet, db: &Db, installed_here: &BTreeSet<String>) -> Result<()> {
    let missing: Vec<&str> = installed_here
        .iter()
        .filter(|name| db.resolve(name).is_none())
        .map(String::as_str)
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let unknown = db.distro.cannot_install_all(&fleet.aur(), &missing)?;
    if let Some((name, why)) = unknown.first() {
        bail!("{name} isn't a package name (a group, a virtual name or a version?). {why}");
    }
    Ok(())
}

pub fn scan(fleet: &Fleet, db: &Db) -> Result<Vec<Item>> {
    let installed_here = fleet.packages();
    let listed = fleet.listed_packages();
    refuse_provided(db, &listed)?;
    refuse_unknown(fleet, db, &installed_here)?;
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

#[cfg(test)]
#[path = "../../tests/scan_packages.rs"]
mod tests;
