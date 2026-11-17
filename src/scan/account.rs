use super::{Item, Kind};
use crate::{cmd, fleet::Fleet};
use anyhow::{Context, Result};
use std::env;

pub fn scan(fleet: &Fleet) -> Result<Vec<Item>> {
    let user = env::var("USER").context("USER is not set")?;
    let mut items = Vec::new();
    if let Some(shell) = &fleet.system.shell {
        let entry = cmd::stdout("getent", &["passwd", &user])?;
        let actual = entry.trim().rsplit(':').next().unwrap_or_default();
        if actual != shell {
            items.push(Item::new(
                Kind::Account,
                format!("shell {actual}"),
                format!("the fleet says {shell}"),
            ));
        }
    }
    let groups = cmd::stdout("id", &["-nG", &user])?;
    let actual: Vec<&str> = groups.split_whitespace().collect();
    let declared = &fleet.system.groups;
    items.extend(
        actual
            .iter()
            .filter(|g| **g != user && !declared.iter().any(|d| d == *g))
            .map(|g| Item::new(Kind::Account, format!("group {g}"), "member, not declared")),
    );
    items.extend(
        declared
            .iter()
            .filter(|d| !actual.contains(&d.as_str()))
            .map(|g| {
                Item::new(
                    Kind::Account,
                    format!("group {g}"),
                    "declared, not a member",
                )
            }),
    );
    Ok(items)
}
