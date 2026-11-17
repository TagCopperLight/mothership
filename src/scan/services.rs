use super::{Item, Kind};
use crate::{cmd, fleet::Fleet};
use anyhow::Result;
use std::path::Path;

pub fn scan(fleet: &Fleet) -> Result<Vec<Item>> {
    let mut items = check(Kind::Service, &[], &fleet.system.services)?;
    items.extend(check(
        Kind::UserService,
        &["--user"],
        &fleet.system.user_services,
    )?);
    Ok(items)
}

/// Units enabled by hand. Units a package enables by default (its preset) don't count.
fn check(kind: Kind, scope: &[&str], declared: &[String]) -> Result<Vec<Item>> {
    let mut args = scope.to_vec();
    args.extend(["list-unit-files", "--state=enabled", "--no-legend"]);
    let listing = cmd::stdout("systemctl", &args)?;
    let enabled: Vec<(&str, &str)> = listing
        .lines()
        .filter_map(|line| {
            let columns: Vec<&str> = line.split_whitespace().collect();
            Some((*columns.first()?, *columns.get(2)?))
        })
        .collect();
    let mut items: Vec<Item> = enabled
        .iter()
        .filter(|(unit, preset)| *preset != "enabled" && !declared.iter().any(|d| d == unit))
        .map(|(unit, _)| Item::new(kind, *unit, "enabled, not declared"))
        .collect();
    items.extend(
        declared
            .iter()
            .filter(|unit| !enabled.iter().any(|(e, _)| e == unit))
            .map(|unit| Item::new(kind, unit, "declared, not enabled")),
    );
    Ok(items)
}

/// What `systemctl enable` creates: the services check reports those, not the file scans.
pub fn is_enablement(path: &Path) -> bool {
    let in_unit_dir = path.parent().is_some_and(|dir| {
        dir == Path::new("/etc/systemd/system")
            || dir == Path::new("/etc/systemd/user")
            || dir.ends_with(".config/systemd/user")
    });
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    in_unit_dir
        && ([".wants", ".requires", ".upholds"]
            .iter()
            .any(|s| name.ends_with(s))
            || path.is_symlink())
}
