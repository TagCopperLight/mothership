mod answers;
mod boot;
mod bootstrap;
mod disk;
mod iso;
mod net;
mod repo;
mod trust;

use crate::{
    apply::{self, ServiceManager},
    cmd::{self, As},
    fleet::{self, Fleet, Machine, Repo},
    host,
    package::{self, Distro, aur},
    prompt::Prompt,
    revert, scan, user,
};
use answers::Flags;
use anyhow::{Context, Result, bail};
use net::Needs;
use std::{env, ffi::OsStr, fs, os::unix::fs::MetadataExt, path::Path};

const ARCHISO: &str = "/run/archiso";
const GH_TOKEN: &str = "MOTHERSHIP_GH_TOKEN";
const FROM_ISO: &str = "MOTHERSHIP_FROM_ISO";

pub fn run(args: &[String]) -> Result<()> {
    let flags = Flags::parse(args)?;
    let mut prompt = Prompt::new();
    let chroot = env::var_os(FROM_ISO).is_some();
    if !chroot && Path::new(ARCHISO).exists() {
        return iso::run(&flags, &mut prompt);
    }
    if let Some(flag) = flags.only_on_the_iso() {
        bail!("{flag} only means something on the Arch ISO");
    }
    here(&flags, &mut prompt, chroot)
}

fn here(flags: &Flags, prompt: &mut Prompt, chroot: bool) -> Result<()> {
    let distro = Distro::here()?;
    if flags.remove_unlisted.is_some() && distro != Distro::Debian {
        bail!("--remove-unlisted only means something on Debian and Ubuntu");
    }
    let token = env::var(GH_TOKEN).ok();
    unsafe { env::remove_var(GH_TOKEN) };
    if !Path::new(fleet::MACHINE).exists() {
        start_server(flags, prompt, distro)?;
    }
    let machine = Machine::load()?;
    let dir = Path::new(fleet::DIR);
    let members = Repo::for_machine(dir.into(), &machine)?.profiles;
    for member in &members {
        if user::by_name(&member.name)?.is_none() {
            revert::create_user(&member.name, member.config.shell.as_deref())?;
        }
    }
    if fs::metadata(dir)?.uid() == 0 {
        let names: Vec<String> = members.iter().map(|m| m.name.clone()).collect();
        adopt(flags, prompt, &names, dir, token.as_deref())?;
    }

    let first_start = !chroot && boot::first_start();
    let answering = first_start.then(boot::answering);
    if first_start {
        net::give_up_waiting_after(boot::NETWORK_WITHIN);
        boot::secure()?;
    }
    trust::keys(prompt, &host::name()?)?;
    drop(answering);
    let fleet = Fleet::load()?;
    match (distro, fleet.system.aur_helper) {
        (Distro::Arch, Some(helper)) if !helper.installed(fleet.admin()) => {
            net::retry("Building the AUR helper", &Needs::url(aur::URL)?, || {
                helper.build(fleet.admin())
            })?;
        }
        _ => {}
    }
    net::retry("Applying the fleet", &Needs::mirror(distro)?, || {
        apply::unasked(match chroot {
            true => ServiceManager::Missing,
            false => ServiceManager::Running,
        })
    })?;
    fleet::require_admin_sudo()?;
    if distro == Distro::Debian {
        let listed = Fleet::load()?.listed_packages();
        package::keep_only(listed, &mut || match flags.remove_unlisted {
            Some(answer) => Ok(answer),
            None => prompt.confirm("Remove them"),
        })?;
    }
    if chroot {
        return boot::setup();
    }
    if first_start {
        boot::finish()?;
    }
    scan::run(None)
}

fn start_server(flags: &Flags, prompt: &mut Prompt, distro: Distro) -> Result<()> {
    let name = match &flags.admin {
        Some(name) => name.clone(),
        None => env::var("SUDO_USER").with_context(|| {
            format!(
                "name the account that will commit the fleet with {}, or run install with sudo \
                 from it",
                answers::ADMIN
            )
        })?,
    };
    let admin = user::by_name(&name)?.with_context(|| format!("there is no user {name}"))?;
    fleet::require_sudo(&admin)?;
    bootstrap::git(distro)?;
    bootstrap::chezmoi(distro)?;
    let dir = Path::new(fleet::DIR);
    if !dir.join(".git").exists() {
        let url = answers::fleet_url(flags, prompt)?;
        fs::create_dir_all(dir.parent().expect("the fleet folder has a parent"))?;
        admin.create_dir_all(dir)?;
        repo::clone(&url, dir, As::User(&admin), distro)?;
    }
    let machine = answers::machine(flags, prompt, dir)?;
    let hostname = answers::hostname(flags, prompt)?;
    host::rename(&hostname)?;
    machine.save(Path::new("/"))
}

fn adopt(
    flags: &Flags,
    prompt: &mut Prompt,
    members: &[String],
    dir: &Path,
    token: Option<&str>,
) -> Result<()> {
    let name = answers::admin(flags, prompt, members)?;
    let admin = user::by_name(&name)?.context("install creates the type's users first")?;
    println!("{name}'s password:");
    let mut tries = 0;
    while let Err(e) = cmd::root("passwd", &[&name]) {
        tries += 1;
        if tries == 3 {
            return Err(e);
        }
    }
    if let Some(token) = token {
        repo::log_in(As::User(&admin), token)?;
    }
    let owner = format!("{}:", admin.name);
    let args = ["-R", "--no-dereference", "--", &owner].map(OsStr::new);
    cmd::root("chown", &[&args[..], &[dir.as_os_str()]].concat())
}
