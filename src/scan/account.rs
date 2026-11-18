use super::{Item, Kind};
use crate::{
    edit::{Add, Key},
    fleet::Fleet,
    revert::Revert,
    user,
};
use anyhow::Result;
use std::fs;

pub fn scan(fleet: &Fleet) -> Result<Vec<Item>> {
    let user = user::current();
    let mut items = Vec::new();
    if let Some(shell) = &fleet.system.shell
        && user.shell != *shell
    {
        items.push(
            Item::new(
                Kind::Account,
                format!("shell {}", user.shell),
                format!("the fleet says {shell}"),
            )
            .with(Add::Shell(user.shell.clone()))
            .reverting(Revert::Shell(shell.clone())),
        );
    }
    // Member lists leave out my own primary group.
    let groups = fs::read_to_string("/etc/group")?;
    let actual: Vec<&str> = groups
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            let members = fields.get(3)?;
            members
                .split(',')
                .any(|m| m == user.name)
                .then_some(fields[0])
        })
        .collect();
    let declared = &fleet.system.groups;
    items.extend(
        actual
            .iter()
            .filter(|g| !declared.iter().any(|d| d == *g))
            .map(|g| {
                Item::new(Kind::Account, format!("group {g}"), "member, not declared")
                    .with(Add::Declare {
                        key: Key::Groups,
                        name: g.to_string(),
                    })
                    .reverting(Revert::Leave(g.to_string()))
            }),
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
                .with(Add::Undeclare {
                    key: Key::Groups,
                    name: g.clone(),
                })
                .reverting(Revert::Join(g.clone()))
            }),
    );
    Ok(items)
}
