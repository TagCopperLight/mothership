use super::{USERS, check_name, machines::Machines, type_dir, types, users};
use anyhow::{Context, Result, bail};
use std::path::Path;

pub fn resolve(
    fleet_dir: &Path,
    machines: &Machines,
    owner: &Path,
    ssh_from: &[String],
) -> Result<Option<Vec<String>>> {
    if ssh_from.is_empty() {
        return Ok(None);
    }
    let shown = owner.display();
    let known = types(fleet_dir)?;
    let mut keys = Vec::new();
    for entry in ssh_from {
        let (type_name, user) = match entry.split_once('/') {
            Some((type_name, user)) => (type_name, Some(user)),
            None => (entry.as_str(), None),
        };
        if !known.iter().any(|name| name == type_name) {
            bail!("{shown}'s ssh_from names {entry}, and the fleet has no type {type_name}");
        }
        let type_dir = type_dir(fleet_dir, type_name);
        let names = match user {
            Some(user) => {
                check_name(user).with_context(|| format!("{shown}'s ssh_from names {entry}"))?;
                if !type_dir.join(USERS).join(user).is_dir() {
                    bail!("{shown}'s ssh_from names {entry}, and {type_name} has no user {user}");
                }
                vec![user.to_owned()]
            }
            None => users(&type_dir)?,
        };
        for user in names {
            for line in machines.logins(type_name, &user)? {
                if !keys.contains(&line) {
                    keys.push(line);
                }
            }
        }
    }
    if keys.is_empty() {
        bail!("{shown}'s ssh_from finds no SSH key in the fleet, so every login would be refused");
    }
    Ok(Some(keys))
}

#[cfg(test)]
#[path = "../../tests/fleet/logins.rs"]
mod tests;
