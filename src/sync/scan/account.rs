use super::Ships;
use crate::sync::item::{Change, Item};
use crate::sync::kind::Kind;
use crate::{
    fleet::edit::{Add, Key},
    fleet::{Fleet, Member},
    sync::revert::{Machine, User},
    system::user,
    util::walk,
};
use anyhow::{Context, Result};
use std::{collections::BTreeSet, fs, path::Path};

const SYSUSERS: [&str; 2] = ["/etc/sysusers.d", "/usr/lib/sysusers.d"];
const LINGER: &str = "/var/lib/systemd/linger";

pub fn users(fleet: &Fleet) -> Result<Vec<Item>> {
    let mut items: Vec<Item> = fleet
        .missing()
        .map(|profile| {
            Item::new(Kind::User, Change::Removed, &profile.name, "missing").reverting(
                Machine::CreateUser {
                    name: profile.name.clone(),
                    shell: profile.config.shell.clone(),
                },
            )
        })
        .collect();
    for person in user::people()? {
        if !fleet.profiles.iter().any(|p| p.name == person.name) {
            items.push(
                Item::new(Kind::User, Change::Added, &person.name, "not in the fleet")
                    .with(Add::ListUser(person.name)),
            );
        }
    }
    Ok(items)
}

pub fn scan(member: &Member, ships: &Ships) -> Result<Vec<Item>> {
    let user = member.user();
    let config = &member.config;
    let mut items = Vec::new();
    let lingers = Path::new(LINGER).join(&user.name).exists();
    if lingers != config.linger {
        let (change, detail) = if lingers {
            (Change::Added, "turned on")
        } else {
            (Change::Removed, "turned off")
        };
        items.push(
            Item::new(Kind::Account, change, "linger", detail)
                .noting("user services run without a login")
                .with(Add::Linger(lingers))
                .reverting(User::Linger(config.linger)),
        );
    }
    if let Some(shell) = &config.shell
        && user.shell != *shell
    {
        let item = Item::new(
            Kind::Account,
            Change::Edited,
            format!("shell {}", user.shell),
            format!("changed from {shell}"),
        )
        .reverting(User::Shell(shell.clone()));
        items.push(if ships.path(Path::new(&user.shell), None)? {
            item.with(Add::Shell(user.shell.clone()))
        } else {
            item.because("no package the fleet lists installs this shell")
        });
    }
    let joined = user::joined(&user.name)?;
    let actual: Vec<&str> = joined.iter().map(|g| g.name.as_str()).collect();
    let declared = &config.groups;
    let created = shipped_groups(ships)?;
    items.extend(
        actual
            .iter()
            .filter(|g| !declared.iter().any(|d| d == *g))
            .map(|g| {
                let item = Item::new(Kind::Account, Change::Added, format!("group {g}"), "joined")
                    .reverting(User::Leave(g.to_string()));
                if created.contains(*g) {
                    item.with(Add::Declare {
                        key: Key::Groups,
                        name: g.to_string(),
                    })
                } else {
                    item.because("no package the fleet lists creates this group")
                }
            }),
    );
    items.extend(
        declared
            .iter()
            .filter(|d| !actual.contains(&d.as_str()))
            .map(|g| {
                Item::new(Kind::Account, Change::Removed, format!("group {g}"), "left")
                    .with(Add::Undeclare {
                        key: Key::Groups,
                        name: g.clone(),
                    })
                    .reverting(User::Join(g.clone()))
            }),
    );
    Ok(items)
}

fn shipped_groups(ships: &Ships) -> Result<BTreeSet<String>> {
    let mut groups: BTreeSet<String> = ships
        .packages
        .iter()
        .flat_map(|package| package.groups.iter().cloned())
        .collect();
    for dir in SYSUSERS {
        for file in walk::children(Path::new(dir))? {
            if ships.path(&file, None)? {
                let text = fs::read_to_string(&file)
                    .with_context(|| format!("reading {}", file.display()))?;
                groups.extend(sysusers_groups(&text));
            }
        }
    }
    Ok(groups)
}

fn sysusers_groups(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            match fields[..] {
                ["u" | "u!", _, id, ..] if id.contains(':') => None,
                ["g" | "u" | "u!", name, ..] => Some(name.to_owned()),
                ["m", _, group, ..] => Some(group.to_owned()),
                _ => None,
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "../../../tests/sync/scan/account.rs"]
mod tests;
