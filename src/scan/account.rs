use super::{Item, Kind};
use crate::{
    edit::{Add, Key},
    fleet::Fleet,
};
use anyhow::{Context, Result};
use std::{env, fs};

// Read directly: getent and id also ask systemd, which stalls while it lists unit files.

pub fn scan(fleet: &Fleet) -> Result<Vec<Item>> {
    let user = env::var("USER").context("USER is not set")?;
    let mut items = Vec::new();
    if let Some(shell) = &fleet.system.shell {
        let passwd = fs::read_to_string("/etc/passwd")?;
        let actual = passwd
            .lines()
            .find(|line| line.split(':').next() == Some(&user))
            .with_context(|| format!("{user} is not in /etc/passwd"))?
            .rsplit(':')
            .next()
            .unwrap_or_default();
        if actual != shell {
            items.push(
                Item::new(
                    Kind::Account,
                    format!("shell {actual}"),
                    format!("the fleet says {shell}"),
                )
                .with(Add::Shell(actual.to_owned())),
            );
        }
    }
    // Member lists leave out my own primary group.
    let groups = fs::read_to_string("/etc/group")?;
    let actual: Vec<&str> = groups
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            let members = fields.get(3)?;
            members.split(',').any(|m| m == user).then_some(fields[0])
        })
        .collect();
    let declared = &fleet.system.groups;
    items.extend(
        actual
            .iter()
            .filter(|g| !declared.iter().any(|d| d == *g))
            .map(|g| {
                Item::new(Kind::Account, format!("group {g}"), "member, not declared").with(
                    Add::Declare {
                        key: Key::Groups,
                        name: g.to_string(),
                    },
                )
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
            }),
    );
    Ok(items)
}
