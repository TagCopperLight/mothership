use super::{Item, Kind};
use crate::{
    cmd,
    fleet::{self, Fleet, Tool},
};
use anyhow::Result;
use std::{collections::BTreeSet, fs};

pub fn scan(fleet: &Fleet) -> Result<Vec<Item>> {
    let tools = &fleet.system.tools;
    let cargo: BTreeSet<String> = cmd::stdout("cargo", &["install", "--list"])?
        .lines()
        .filter(|line| !line.starts_with(char::is_whitespace))
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .collect();
    let uv: BTreeSet<String> = cmd::stdout("uv", &["tool", "list"])?
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('-'))
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .collect();

    let mut items = compare("cargo", &tools.cargo, &cargo);
    items.extend(compare("uv", &tools.uv, &uv));
    items.extend(compare("bun", &tools.bun, &bun_globals()?));
    items.extend(
        fleet
            .system
            .installers
            .iter()
            .filter(|installer| !fleet::expand(&installer.creates).exists())
            .map(|installer| {
                Item::new(
                    Kind::Tool,
                    installer.name.as_str(),
                    "declared, not installed",
                )
            }),
    );
    Ok(items)
}

fn compare(manager: &str, declared: &[Tool], installed: &BTreeSet<String>) -> Vec<Item> {
    let declared: BTreeSet<&str> = declared.iter().map(Tool::name).collect();
    let mut items: Vec<Item> = installed
        .iter()
        .filter(|name| !declared.contains(name.as_str()))
        .map(|name| {
            Item::new(
                Kind::Tool,
                format!("{manager} {name}"),
                "installed, not declared",
            )
        })
        .collect();
    items.extend(
        declared
            .iter()
            .filter(|name| !installed.contains(**name))
            .map(|name| {
                Item::new(
                    Kind::Tool,
                    format!("{manager} {name}"),
                    "declared, not installed",
                )
            }),
    );
    items
}

/// bun writes this file on the first `bun add -g`, so no file means no globals.
fn bun_globals() -> Result<BTreeSet<String>> {
    let path = fleet::home().join(".bun/install/global/package.json");
    if !path.exists() {
        return Ok(BTreeSet::new());
    }
    let manifest: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    Ok(manifest["dependencies"]
        .as_object()
        .map(|deps| deps.keys().cloned().collect())
        .unwrap_or_default())
}
