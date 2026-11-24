mod answers;
mod boot;
mod bootstrap;
mod disk;
mod iso;
mod net;
mod repo;
mod trust;

use crate::{
    fleet::{self, Fleet, Here, Repo, machines::Machines},
    package::{self, Distro},
    services,
    sync::apply::{self, ServiceManager},
    sync::revert,
    sync::scan,
    system::host,
    system::user::{self, User},
    trust::github,
    trust::ssh,
    util::cmd,
    util::file,
    util::prompt::Prompt,
};
use answers::Flags;
use anyhow::{Context, Result, anyhow, bail};
use ssh_key::PrivateKey;
use std::{env, ffi::OsStr, fs, os::unix::fs::MetadataExt, path::Path};

const ARCHISO: &str = "/run/archiso";
const FROM_ISO: &str = "MOTHERSHIP_FROM_ISO";
const CARRIED_KEY: &str = "/var/lib/mothership/install-key";

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
    let started;
    let flags = match Path::new(fleet::MACHINE).exists() {
        true => flags,
        false => {
            started = Flags {
                admin: Some(start_server(flags, prompt, distro)?),
                ..flags.clone()
            };
            &started
        }
    };
    let dir = Path::new(fleet::DIR);
    let machine = Here::load()?.machine(&Machines::load(dir)?)?;
    let members = Repo::for_machine(dir.into(), &machine)?.profiles;
    for member in &members {
        if user::by_name(&member.name)?.is_none() {
            revert::create_user(&member.name, member.config.shell.as_deref())?;
        }
    }
    if fs::metadata(dir)?.uid() == 0 {
        let names: Vec<String> = members.iter().map(|m| m.name.clone()).collect();
        adopt(flags, prompt, &names, dir)?;
    }
    take_carried_key(&fleet::admin()?)?;

    let first_start = !chroot && boot::first_start();
    if first_start {
        boot::secure()?;
    }
    trust::keys(prompt)?;
    let fleet = Fleet::load()?;
    Here::load()?.settle(&fleet.machines)?;
    match (distro, fleet.system.aur_helper) {
        (Distro::Arch, Some(helper)) if !helper.installed(fleet.admin()) => {
            helper.build(fleet.admin())?;
        }
        _ => {}
    }
    services::ask_republished(&fleet, &mut |question| prompt.confirm(question))?;
    apply::unasked(match chroot {
        true => ServiceManager::Missing,
        false => ServiceManager::Running,
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

fn start_server(flags: &Flags, prompt: &mut Prompt, distro: Distro) -> Result<String> {
    bootstrap::git(distro)?;
    bootstrap::chezmoi(distro)?;
    let hostname = answers::hostname(flags, prompt)?;
    let admin = answers::admin_name(flags, prompt)?;
    let dir = Path::new(fleet::DIR);
    if !dir.join(".git").exists() {
        let url = answers::fleet_url(flags, prompt)?;
        let owner = match user::by_name(&admin)? {
            Some(existing) => existing,
            None => user::by_name("root")?.context("the system has a root user")?,
        };
        fs::create_dir_all(dir.parent().expect("the fleet folder has a parent"))?;
        owner.create_dir_all(dir)?;
        let title = github::title(&hostname, &admin);
        repo::clone(&url, dir, &owner, &title, distro, prompt)?;
        if owner.uid == 0 {
            hand_over_root_key(&owner)?;
        }
    }
    let machine = answers::machine(flags, prompt, dir)?;
    answers::check_admin(dir, &machine, &admin)?;
    host::rename(&hostname)?;
    Here::joining(&hostname, machine).save(Path::new("/"))?;
    Ok(admin)
}

fn hand_over_root_key(root: &User) -> Result<()> {
    let key = ssh::key_path(root);
    let Some(bytes) = file::read(&key)? else {
        return Ok(());
    };
    let carried = Path::new(CARRIED_KEY);
    file::remove_if_found(carried)?;
    fs::create_dir_all(carried.parent().expect("the carried key has a folder"))?;
    file::create_private(carried, &bytes)?;
    file::remove_if_found(&key)?;
    file::remove_if_found(&ssh::public_path(root)).map(drop)
}

fn adopt(flags: &Flags, prompt: &mut Prompt, members: &[String], dir: &Path) -> Result<()> {
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
    let owner = format!("{}:", admin.name);
    let args = ["-R", "--no-dereference", "--", &owner].map(OsStr::new);
    cmd::root("chown", &[&args[..], &[dir.as_os_str()]].concat())
}

fn take_carried_key(admin: &User) -> Result<()> {
    let carried = Path::new(CARRIED_KEY);
    let Some(bytes) = file::read_root_only(carried)? else {
        return Ok(());
    };
    if ssh::user_key(admin)?.is_none() {
        let key = PrivateKey::from_openssh(bytes).map_err(|e| anyhow!("{CARRIED_KEY}: {e}"))?;
        ssh::write_key(admin, &key)?;
    }
    fs::remove_file(carried).with_context(|| format!("removing {CARRIED_KEY}"))
}
