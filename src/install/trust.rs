use crate::{
    fleet::{
        self, Fleet,
        checked::{self, Advance, Anchor, Outcome, Remote},
    },
    home::snapshot::Snapshot,
    secrets,
    trust::github,
    trust::sign,
    util::cmd::As,
    util::prompt::Prompt,
};
use anyhow::{Context, Result, bail};
use std::{ffi::OsStr, fs, path::Path};

const GITCONFIG: &str = ".gitconfig";

#[derive(Clone, Copy)]
enum Next {
    Code,
    Recovery,
}

pub fn keys(prompt: &mut Prompt, hostname: &str) -> Result<()> {
    let admin = fleet::admin()?;
    loop {
        let anchored = checked::anchored()?;
        let fleet = match anchored {
            true => Fleet::load()?,
            false => Fleet::load_at(Path::new(fleet::DIR))?,
        };
        crate::trust::create_keys(&fleet)?;
        let command = crate::trust::trust_command(&fleet, hostname)?;
        match &command {
            None if anchored => return Ok(()),
            None => println!("This machine needs a fleet commit to trust first."),
            Some(command) => {
                println!("Run this on a machine that signs for the fleet:");
                println!("  {command}");
            }
        }
        let answers = [('c', Next::Code), ('r', Next::Recovery)];
        match prompt.ask(
            "c type the code it prints · r type the recovery key ›",
            &answers,
        )? {
            Some(Next::Code) => {
                let code = prompt.line("Code:")?;
                if let Err(e) = checked::sync_from(&admin, Remote::Fetch, Anchor::Code(&code)) {
                    println!("That code doesn't work: {e:#}");
                }
            }
            Some(Next::Recovery) => recover(prompt, hostname, &fleet, anchored)?,
            None => bail!("install stopped before the fleet trusted this machine"),
        }
    }
}

fn recover(prompt: &mut Prompt, hostname: &str, fleet: &Fleet, anchored: bool) -> Result<()> {
    let age = secrets::identity(&prompt.hidden("Recovery key:")?)?;
    if secrets::public(&age) != fleet.recovery.key {
        bail!(
            "that isn't the recovery key the fleet's {} names",
            fleet::MANIFEST
        );
    }
    let admin = fleet.admin();
    if !anchored {
        println!("The fleet's newest commits:");
        for line in checked::newest(admin, 5)? {
            println!("  {line}");
        }
        if !prompt.confirm("Trust the fleet as it is")? {
            bail!("install stopped before the fleet trusted this machine");
        }
        checked::sync_from(admin, Remote::Skip, Anchor::Tip)?;
    }
    let Some(stage) = checked::stage(prompt)? else {
        bail!("install stopped before the fleet trusted this machine");
    };
    let staged = Fleet::load_at(&stage.dir)?;
    let entries = crate::trust::pending(&staged)?;
    let mut keys = staged.keys().clone();
    keys.push(age.clone());
    let uploads = crate::trust::trust_host(
        &staged,
        &keys,
        hostname,
        &staged.type_name,
        &entries,
        prompt,
    )?;
    let who = author(&staged, prompt)?;
    let message = format!("Trust {hostname} as {}", staged.type_name);
    let key = sign::recovery_key(&age);
    if let Outcome::Declined = stage.commit(admin, &who, &message, &key, prompt, Advance::Merge)? {
        bail!("install stopped before the fleet trusted this machine");
    }
    github::add(admin, &uploads)?;
    checked::sync(admin, Remote::Skip).map(drop)
}

fn author(fleet: &Fleet, prompt: &mut Prompt) -> Result<String> {
    let admin = fleet.admin();
    if let Some(who) = checked::author_if_set(admin)? {
        return Ok(who);
    }
    if let Some(who) = fleet_author(fleet)? {
        return Ok(who);
    }
    let name = prompt.line("Your name, for the fleet's commits:")?;
    let email = prompt.line("Your email, for the fleet's commits:")?;
    Ok(format!("{name} <{email}>"))
}

fn fleet_author(fleet: &Fleet) -> Result<Option<String>> {
    let admin = fleet.admin();
    let member = fleet
        .members
        .iter()
        .find(|member| member.name == admin.name)
        .with_context(|| format!("{} lists no user {}", fleet.type_name, admin.name))?;
    let root = member.chezmoi_root(fleet);
    let empty = Snapshot::take(admin, &[])?;
    let target = admin.home.join(GITCONFIG);
    let managed = root.output(&empty, &[OsStr::new("managed"), target.as_os_str()])?;
    if managed.trim_ascii().is_empty() {
        return Ok(None);
    }
    let config = tempfile::NamedTempFile::new()?;
    fs::write(
        config.path(),
        root.output(&empty, &[OsStr::new("cat"), target.as_os_str()])?,
    )?;
    let path = config.path().to_string_lossy();
    checked::author_from(|key| checked::git_config(As::Root, &["config", "--file", &path, key]))
}
