use super::repo;
use crate::{
    cmd::As,
    fleet::{self, Fleet},
    prompt::Prompt,
    secrets::{self, MachineKey},
};
use anyhow::{Result, bail};
use std::{collections::BTreeMap, path::Path};

pub fn keys(prompt: &mut Prompt, hostname: &str) -> Result<()> {
    loop {
        let fleet = Fleet::load()?;
        let mut waiting = Vec::new();
        for key in secrets::machine_keys(&fleet)? {
            if key.trusted {
                continue;
            }
            if secrets::names(&key.list)?.is_empty() {
                secrets::trust(&fleet, &key.target, hostname, &key.public)?;
            } else {
                waiting.push(key);
            }
        }
        if waiting.is_empty() {
            return fleet.hand_back();
        }
        let (question, answers): (&str, &[char]) = match print(&waiting, hostname)? {
            true => (
                "d done, pull the fleet · r type the recovery key ›",
                &['d', 'r'],
            ),
            false => ("r type the recovery key ›", &['r']),
        };
        match prompt.ask(question, answers)? {
            Some('d') => repo::pull(Path::new(fleet::DIR), None, || {
                As::User(fleet.admin()).command("git", &[])
            })?,
            Some('r') => {
                let mut keys = fleet.keys().clone();
                let public = keys.add(&prompt.hidden("Recovery key:")?)?;
                for key in &waiting {
                    if !secrets::holds(&key.list, &public)? {
                        bail!("that key isn't in {}", key.list.display());
                    }
                }
                for key in &waiting {
                    secrets::trust_with(&fleet, &keys, &key.target, hostname, &key.public)?;
                }
                fleet.hand_back()?;
            }
            _ => bail!("install stopped before the fleet trusted this machine's keys"),
        }
    }
}

fn print(waiting: &[MachineKey], hostname: &str) -> Result<bool> {
    let mut by_machine: BTreeMap<String, Vec<&MachineKey>> = BTreeMap::new();
    for key in waiting {
        for name in secrets::names(&key.list)? {
            if name != secrets::RECOVERY && name != hostname {
                by_machine.entry(name).or_default().push(key);
            }
        }
    }
    if by_machine.is_empty() {
        println!(
            "The fleet doesn't trust this machine's keys yet, and only the recovery key opens them."
        );
        return Ok(false);
    }
    println!("The fleet doesn't trust this machine's keys yet. Run these, then commit and push:");
    for (machine, keys) in by_machine {
        println!("  On {machine}:");
        for key in keys {
            println!(
                "    {}",
                secrets::trust_command(&key.target, hostname, &key.public)
            );
        }
    }
    Ok(true)
}
