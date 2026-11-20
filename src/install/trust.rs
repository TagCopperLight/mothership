use super::repo;
use crate::{
    cmd::As,
    fleet::{self, Fleet},
    prompt::Prompt,
    secrets,
    trust::MachineKey,
};
use anyhow::{Result, bail};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Copy)]
enum Next {
    Pull,
    Recovery,
}

pub fn keys(prompt: &mut Prompt, hostname: &str) -> Result<()> {
    crate::trust::create_keys(&Fleet::load()?)?;
    loop {
        let fleet = Fleet::load()?;
        let mut waiting = Vec::new();
        for key in crate::trust::machine_keys(&fleet)? {
            if key.trusted {
                continue;
            }
            if !crate::trust::guards_secrets(&fleet, &key.target)? {
                crate::trust::trust(&fleet, &key.target, hostname, &key.public, prompt)?;
            } else {
                waiting.push(key);
            }
        }
        if waiting.is_empty() {
            return fleet.hand_back();
        }
        let (question, answers): (&str, &[(char, Next)]) = match print(&waiting, hostname)? {
            true => (
                "d done, pull the fleet · r type the recovery key ›",
                &[('d', Next::Pull), ('r', Next::Recovery)],
            ),
            false => ("r type the recovery key ›", &[('r', Next::Recovery)]),
        };
        match prompt.ask(question, answers)? {
            Some(Next::Pull) => repo::pull(Path::new(fleet::DIR), None, || {
                As::User(fleet.admin()).command::<&str>("git", &[])
            })?,
            Some(Next::Recovery) => {
                let mut keys = fleet.keys().clone();
                let public = keys.add(&prompt.hidden("Recovery key:")?)?;
                if public != fleet.recovery.key {
                    bail!(
                        "that isn't the recovery key the fleet's {} names",
                        fleet::MANIFEST
                    );
                }
                for key in &waiting {
                    crate::trust::trust_with(
                        &fleet,
                        &keys,
                        &key.target,
                        hostname,
                        &key.public,
                        prompt,
                    )?;
                }
                fleet.hand_back()?;
            }
            None => bail!("install stopped before the fleet trusted this machine's keys"),
        }
    }
}

fn print(waiting: &[MachineKey], hostname: &str) -> Result<bool> {
    let mut by_machine: BTreeMap<String, Vec<&MachineKey>> = BTreeMap::new();
    for key in waiting {
        for name in secrets::names(&key.list)? {
            if name != hostname {
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
                crate::trust::trust_command(&key.target, hostname, &key.public)
            );
        }
    }
    Ok(true)
}
