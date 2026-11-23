use super::{USERS, type_dir, types};
use crate::{secrets, trust::ssh, util::walk};
use anyhow::{Result, bail};
use std::path::Path;

pub fn resolve(fleet_dir: &Path, owner: &Path, ssh_from: &[String]) -> Result<Option<Vec<String>>> {
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
        let users = type_dir(fleet_dir, type_name).join(USERS);
        let dirs = match user {
            Some(user) => {
                let dir = users.join(user);
                if user.contains('/') || matches!(user, "" | "." | "..") || !dir.is_dir() {
                    bail!("{shown}'s ssh_from names {entry}, and {type_name} has no user {user}");
                }
                vec![dir]
            }
            None => walk::children(&users)?,
        };
        for dir in dirs {
            let user = dir.file_name().unwrap_or_default().to_string_lossy();
            for host in secrets::read(&dir.join(ssh::KEYS))? {
                let line = format!("{} {}/{user}", ssh::normalize(&host.key)?, host.name);
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
