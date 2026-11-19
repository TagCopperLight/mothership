use super::{BINARY, FROM_ISO, Flags, GH_TOKEN, ask_hostname, ask_machine, boot, clone, disk, git};
use crate::{
    cmd::{self, As},
    fleet::{self, Machine},
    net,
    package::Distro,
    prompt::Prompt,
    walk,
};
use anyhow::{Context, Result, bail};
use std::{
    env, fs,
    io::ErrorKind,
    os::unix::{self, fs::PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};

const FLEET: &str = "/tmp/fleet";

pub fn run(flags: &Flags, prompt: &mut Prompt) -> Result<()> {
    let resumed = disk::still_open()?;
    if resumed {
        println!("The disk from the last run is still open: install goes on from there.");
    } else {
        boot::check_firmware()?;
    }
    net::wait();
    net::retry("Installing git", || {
        cmd::root("pacman", &["-Sy"])?;
        Distro::Arch.bootstrap(&["git"])
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
        load_keymap(dir, &machine)?;
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
    let copy = walk::under(root, fleet::DIR);
    if copy.exists() {
        let path = copy.to_string_lossy();
        let args = ["-c", "safe.directory=*", "-C", &path, "pull", "--ff-only"];
        net::retry("Pulling the fleet", || git(As::Root, &args))?;
        cmd::root(
            "chown",
            &["-R", "--no-dereference", "--reference", &path, &path],
        )?;
    } else {
        let parent = copy.parent().expect("the fleet folder has a parent");
        fs::create_dir_all(parent)?;
        let partial = parent.join(".fleet");
        if partial.exists() {
            fs::remove_dir_all(&partial)?;
        }
        cmd::root("cp", &["-a", FLEET, &partial.to_string_lossy()])?;
        fs::rename(&partial, &copy)?;
    }
    let binary = walk::under(root, BINARY);
    fs::copy(env::current_exe()?, &binary)?;
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))?;

    let args = [disk::ROOT, BINARY, "install"];
    let mut chroot = Command::new("arch-chroot");
    chroot.args(args).env(FROM_ISO, "1");
    if private {
        let token = cmd::stdout(As::Root, "gh", &["auth", "token"])?;
        chroot.env(GH_TOKEN, token.trim());
    }
    cmd::status(chroot, "arch-chroot", &args)?;
    disk::close()?;
    println!(
        "The system is installed. Take out the USB stick and restart: type the disk's \
         passphrase, and install finishes there."
    );
    Ok(())
}

fn load_keymap(dir: &Path, machine: &Machine) -> Result<()> {
    let path = dir
        .join("types")
        .join(&machine.device_type)
        .join("etc/vconsole.conf");
    let text = match fs::read_to_string(&path) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        text => text.with_context(|| format!("reading {}", path.display()))?,
    };
    match text.lines().find_map(|line| line.strip_prefix("KEYMAP=")) {
        Some(keymap) => cmd::root("loadkeys", &[keymap.trim_matches('"')]),
        None => Ok(()),
    }
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
