use super::{
    FROM_ISO, GH_TOKEN,
    answers::{self, ADMIN, Flags},
    boot::{self, BINARY},
    disk,
    net::{self, Needs},
    repo,
};
use crate::{
    cmd::{self, As, Cmd},
    file,
    fleet::{self, Repo},
    package::Distro,
    prompt::Prompt,
};
use anyhow::Result;
use std::{
    env, fs,
    os::unix::{
        self,
        fs::{MetadataExt, PermissionsExt},
    },
    path::Path,
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
    let mirror = Needs::mirror(Distro::Arch)?;
    if !cmd::found(As::Root, "git") {
        net::retry("Installing git", &mirror, || {
            cmd::root("pacman", &["-Sy", "--needed", "--noconfirm", "git"])
        })?;
    }
    let url = answers::fleet_url(flags, prompt)?;
    let dir = Path::new(FLEET);
    fs::create_dir_all(dir)?;
    let private = repo::clone(&url, dir, As::Root, Distro::Arch)?;
    let machine = answers::machine(flags, prompt, dir)?;
    let fleet = Repo::for_machine(dir.into(), &machine)?;
    boot::check_type(&fleet)?;
    let members: Vec<String> = fleet.profiles.iter().map(|m| m.name.clone()).collect();
    answers::admin_given(flags, &members)?;
    let hostname = answers::hostname(flags, prompt)?;
    let timezone = answers::timezone(flags, prompt)?;
    if !resumed {
        let target = disk::choose(flags, prompt)?;
        if let Some(keymap) = boot::keymap(&fleet)? {
            cmd::root("loadkeys", &[&keymap])?;
        }
        disk::prepare(&target, prompt)?;
    }

    let mut packages = vec!["base", "base-devel", "git", "chezmoi"];
    if private {
        packages.push(Distro::Arch.gh_package());
    }
    let args = [&["-K", disk::ROOT], &packages[..]].concat();
    net::retry("Installing the base system", &mirror, || {
        cmd::root("pacstrap", &args)
    })?;
    let root = Path::new(disk::ROOT);
    file::write(
        &root.join("etc/hostname"),
        format!("{hostname}\n").as_bytes(),
        0o644,
    )?;
    let localtime = root.join("etc/localtime");
    file::link(&localtime, &answers::zoneinfo(&timezone))?;
    machine.save(root)?;
    let token = match private {
        true => Some(
            cmd::stdout(As::Root, "gh", &["auth", "token"])?
                .trim()
                .to_owned(),
        ),
        false => None,
    };
    let copy = file::under(root, fleet::DIR);
    if copy.exists() {
        let origin = repo::origin(dir, || As::Root.command::<&str>("git", &[]))?;
        pull_as_owner(&origin, &copy, token.as_deref())?;
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
    let binary = file::under(root, BINARY);
    fs::copy(env::current_exe()?, &binary)?;
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))?;

    let mut args = vec![disk::ROOT, BINARY, "install"];
    if let Some(admin) = &flags.admin {
        args.extend([ADMIN, admin]);
    }
    let mut chroot = Command::new("arch-chroot");
    chroot.args(&args).env(FROM_ISO, "1");
    if let Some(token) = &token {
        chroot.env(GH_TOKEN, token);
    }
    cmd::status(chroot, "arch-chroot", &args)?;
    disk::close()?;
    println!(
        "The system is installed. Take out the USB stick and restart: type the disk's \
         passphrase, and install finishes there."
    );
    Ok(())
}

fn pull_as_owner(origin: &str, copy: &Path, token: Option<&str>) -> Result<()> {
    let meta = fs::metadata(copy)?;
    let (uid, gid) = (meta.uid().to_string(), meta.gid().to_string());
    let home = tempfile::tempdir()?;
    unix::fs::chown(home.path(), Some(meta.uid()), Some(meta.gid()))?;
    repo::pull(copy, Some(origin), || {
        let mut git = Command::new("setpriv");
        git.args([
            "--reuid",
            &uid,
            "--regid",
            &gid,
            "--clear-groups",
            "--",
            "git",
        ])
        .env("HOME", home.path());
        if let Some(token) = token {
            git.env("GH_TOKEN", token);
        }
        Cmd::from(git)
    })
}
