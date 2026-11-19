use super::{FROM_ISO, Flags, GH_TOKEN, ask_hostname, ask_machine, boot, clone, disk};
use crate::{
    cmd::{self, As},
    fleet, net,
    package::Distro,
    prompt::Prompt,
};
use anyhow::{Context, Result, bail};
use std::{
    env, fs,
    os::unix::{self, fs::PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};

const FLEET: &str = "/tmp/fleet";

/// On the Arch ISO: asks everything, prepares the disk and installs the base system, then
/// finishes from inside it. Every step after the disk's can run twice, so a run that stopped
/// after it goes on from there.
pub fn run(flags: &Flags, prompt: &mut Prompt) -> Result<()> {
    let resumed = disk::still_open()?;
    if resumed {
        println!("The disk from the last run is still open: install goes on from there.");
    } else {
        boot::check_firmware()?;
    }
    net::wait();
    net::retry("Installing git", || {
        cmd::root("pacman", &["-Sy", "--needed", "--noconfirm", "git"])
    })?;
    let url = match &flags.fleet {
        Some(url) => url.clone(),
        None => prompt.line("Fleet repository:")?,
    };
    let dir = Path::new(FLEET);
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    fs::create_dir_all(dir)?;
    let private = clone(&url, dir, As::Root, Distro::Arch)?;
    let machine = ask_machine(flags, prompt, dir)?;
    let hostname = ask_hostname(flags, prompt)?;
    let timezone = ask_timezone(flags, prompt)?;
    if !resumed {
        let target = disk::choose(flags, prompt)?;
        disk::prepare(&target, prompt)?;
    }

    let mut packages = vec!["base", "base-devel", "git", "chezmoi"];
    if private {
        packages.push("github-cli");
    }
    let args = [&["-K", disk::ROOT], &packages[..]].concat();
    net::retry("Installing the base system", || {
        cmd::root("pacstrap", &args)
    })?;
    let root = Path::new(disk::ROOT);
    fs::write(root.join("etc/hostname"), format!("{hostname}\n"))?;
    let localtime = root.join("etc/localtime");
    if fs::symlink_metadata(&localtime).is_ok() {
        fs::remove_file(&localtime)?;
    }
    unix::fs::symlink(zoneinfo(&timezone), &localtime)?;
    machine.save(root)?;
    // The new system's copy holds what an earlier run did in it, such as keys it trusted.
    let copy = root.join(fleet::DIR.trim_start_matches('/'));
    if !copy.exists() {
        let parent = copy.parent().context("the fleet folder has a parent")?;
        fs::create_dir_all(parent)?;
        let partial = parent.join(".fleet");
        if partial.exists() {
            fs::remove_dir_all(&partial)?;
        }
        cmd::root("cp", &["-a", FLEET, &partial.to_string_lossy()])?;
        fs::rename(&partial, &copy)?;
    }
    let binary = root.join("usr/local/bin/mothership");
    fs::copy(env::current_exe()?, &binary)?;
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))?;

    let mut chroot = Command::new("arch-chroot");
    chroot
        .args([disk::ROOT, "mothership", "install"])
        .env(FROM_ISO, "1");
    if private {
        // gh reads its own config here, with no network.
        let token = cmd::stdout(As::Root, "gh", &["auth", "token"])?;
        chroot.env(GH_TOKEN, token.trim());
    }
    cmd::status(
        chroot,
        "arch-chroot",
        &[disk::ROOT, "mothership", "install"],
    )?;
    disk::close()?;
    println!(
        "The system is installed. Take out the USB stick and restart: type the disk's \
         passphrase, and install finishes there."
    );
    Ok(())
}

fn zoneinfo(timezone: &str) -> PathBuf {
    Path::new("/usr/share/zoneinfo").join(timezone)
}

fn ask_timezone(flags: &Flags, prompt: &mut Prompt) -> Result<String> {
    let known = |name: &str| !name.starts_with('/') && zoneinfo(name).is_file();
    match &flags.timezone {
        Some(name) if known(name) => Ok(name.clone()),
        Some(name) => bail!("{name} isn't a time zone, such as Europe/Paris"),
        None => loop {
            let name = prompt.line("Time zone, such as Europe/Paris:")?;
            if known(&name) {
                return Ok(name);
            }
            println!("{name} isn't in /usr/share/zoneinfo.");
        },
    }
}
