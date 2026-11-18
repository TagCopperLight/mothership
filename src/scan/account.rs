use super::{Item, Kind, Ships};
use crate::{
    edit::{Add, Key},
    fleet::Member,
    revert::Revert,
};
use anyhow::{Context, Result};
use std::{
    collections::BTreeSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

/// Packages create their groups through these, systemd's basic ones included.
const SYSUSERS: [&str; 2] = ["/etc/sysusers.d", "/usr/lib/sysusers.d"];
/// `loginctl enable-linger` leaves a file named after the user here.
const LINGER: &str = "/var/lib/systemd/linger";

pub fn scan(member: &Member, ships: &Ships) -> Result<Vec<Item>> {
    let user = member.user();
    let config = &member.config;
    let mut items = Vec::new();
    let lingers = Path::new(LINGER).join(&user.name).exists();
    if lingers != config.linger {
        let detail = if lingers {
            "on, not declared"
        } else {
            "declared, off"
        };
        items.push(
            Item::new(Kind::Account, "linger", detail)
                .with(Add::Linger(lingers))
                .reverting(Revert::Linger(config.linger)),
        );
    }
    if let Some(shell) = &config.shell
        && user.shell != *shell
    {
        let item = Item::new(
            Kind::Account,
            format!("shell {}", user.shell),
            format!("the fleet says {shell}"),
        )
        .reverting(Revert::Shell(shell.clone()));
        items.push(if ships.path(Path::new(&user.shell), None)? {
            item.with(Add::Shell(user.shell.clone()))
        } else {
            item.because("no package the fleet lists installs this shell")
        });
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
    let declared = &config.groups;
    let created = shipped_groups(ships)?;
    items.extend(
        actual
            .iter()
            .filter(|g| !declared.iter().any(|d| d == *g))
            .map(|g| {
                let item = Item::new(Kind::Account, format!("group {g}"), "member, not declared")
                    .reverting(Revert::Leave(g.to_string()));
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

/// The groups the sysusers files of the fleet's packages and etc/ create.
fn shipped_groups(ships: &Ships) -> Result<BTreeSet<String>> {
    let mut groups = BTreeSet::new();
    for dir in SYSUSERS {
        let entries = match fs::read_dir(dir) {
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            entries => entries.with_context(|| format!("reading {dir}"))?,
        };
        let files: Vec<PathBuf> = entries
            .map(|entry| Ok(entry?.path()))
            .collect::<Result<_>>()?;
        for file in files {
            if ships.path(&file, None)? {
                groups.extend(sysusers_groups(&fs::read_to_string(&file)?));
            }
        }
    }
    Ok(groups)
}

/// `g name`, `u name` (a user with its own group) and `m user group` each create a group.
fn sysusers_groups(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            match fields[..] {
                ["g" | "u", name, ..] => Some(name.to_owned()),
                ["m", _, group, ..] => Some(group.to_owned()),
                _ => None,
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "../../tests/account.rs"]
mod tests;
