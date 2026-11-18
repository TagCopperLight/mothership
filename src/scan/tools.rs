use super::{Item, Kind};
use crate::{
    cmd,
    edit::{Add, Key},
    fleet::{self, Fleet, Manager},
    record::Record,
    revert::Revert,
    user,
};
use anyhow::Result;
use std::{collections::BTreeSet, fs};

pub fn scan(fleet: &Fleet) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    for manager in Manager::ALL {
        items.extend(compare(fleet, manager, &installed(manager)?));
    }
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
                .with(Add::UndeclareInstaller(installer.name.clone()))
                .reverting(Revert::RunInstaller(installer.name.clone()))
            }),
    );
    Ok(items)
}

/// Tools the fleet moved to another source since the last apply. The scan can't see this: it
/// only compares names, since the managers don't all tell where a tool came from.
pub fn moved(fleet: &Fleet, record: &Record, scanned: &[Item]) -> Vec<Item> {
    let mut items = Vec::new();
    for manager in Manager::ALL {
        for tool in fleet.system.tools.of(manager) {
            let subject = subject(manager, tool.name());
            let missing = scanned.iter().any(|item| item.subject == subject);
            if !missing
                && record
                    .tool(manager, tool.name())
                    .is_some_and(|old| old != tool)
            {
                items.push(
                    Item::new(Kind::Tool, subject, "the fleet changed its source").reverting(
                        Revert::ReinstallTool {
                            manager,
                            tool: tool.clone(),
                        },
                    ),
                );
            }
        }
    }
    items
}

fn subject(manager: Manager, name: &str) -> String {
    format!("{} {name}", manager.name())
}

fn compare(fleet: &Fleet, manager: Manager, installed: &BTreeSet<String>) -> Vec<Item> {
    let declared = fleet.system.tools.of(manager);
    let key = Key::Tools(manager);
    let mut items: Vec<Item> = installed
        .iter()
        .filter(|name| !declared.iter().any(|tool| tool.name() == *name))
        .map(|name| {
            Item::new(
                Kind::Tool,
                subject(manager, name),
                "installed, not declared",
            )
            .with(Add::Declare {
                key,
                name: name.clone(),
            })
            .reverting(Revert::UninstallTool {
                manager,
                name: name.clone(),
            })
        })
        .collect();
    items.extend(
        declared
            .iter()
            .filter(|tool| !installed.contains(tool.name()))
            .map(|tool| {
                Item::new(
                    Kind::Tool,
                    subject(manager, tool.name()),
                    "declared, not installed",
                )
                .with(Add::Undeclare {
                    key,
                    name: tool.name().to_owned(),
                })
                .reverting(Revert::InstallTool {
                    manager,
                    tool: tool.clone(),
                })
            }),
    );
    items
}

fn installed(manager: Manager) -> Result<BTreeSet<String>> {
    Ok(match manager {
        Manager::Cargo => cmd::stdout("cargo", &["install", "--list"])?
            .lines()
            .filter(|line| !line.starts_with(char::is_whitespace))
            .filter_map(|line| line.split_whitespace().next())
            .map(str::to_owned)
            .collect(),
        Manager::Uv => cmd::stdout("uv", &["tool", "list"])?
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('-'))
            .filter_map(|line| line.split_whitespace().next())
            .map(str::to_owned)
            .collect(),
        Manager::Bun => bun_globals()?,
    })
}

/// bun writes this file on the first `bun add -g`, so no file means no globals.
fn bun_globals() -> Result<BTreeSet<String>> {
    let path = user::home().join(".bun/install/global/package.json");
    if !path.exists() {
        return Ok(BTreeSet::new());
    }
    let manifest: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    Ok(manifest["dependencies"]
        .as_object()
        .map(|deps| deps.keys().cloned().collect())
        .unwrap_or_default())
}
