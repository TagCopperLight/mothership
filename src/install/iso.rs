use super::{
    CARRIED_KEY, FROM_ISO,
    answers::{self, ADMIN, Flags},
    boot, disk,
    net::{self, Needs},
    repo,
};
use crate::trust::update::BINARY;
use crate::{
    fleet::{self, Repo, checked},
    package::Distro,
    system::user,
    trust::ssh,
    util::cmd::{self, As},
    util::file,
    util::prompt::Prompt,
};
use anyhow::{Context, Result, bail};
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
    let hostname = answers::hostname(flags, prompt)?;
    let admin = match &flags.admin {
        Some(name) => name.clone(),
        None => prompt.line("Admin, who commits and pushes the fleet:")?,
    };
    let dir = Path::new(FLEET);
    fs::create_dir_all(dir)?;
    let root_user = user::by_name("root")?.context("the ISO has a root user")?;
    let title = crate::trust::github::title(&hostname, &admin);
    repo::clone(&url, dir, &root_user, &title, Distro::Arch, prompt)?;
    let machine = answers::machine(flags, prompt, dir)?;
    let fleet = Repo::for_machine(dir.into(), &machine)?;
    boot::check_type(&fleet)?;
    if !fleet.profiles.iter().any(|member| member.name == admin) {
        bail!("{} has no user {admin}", machine.device_type);
    }
    let timezone = answers::timezone(flags, prompt)?;
    if !resumed {
        let target = disk::choose(flags, prompt)?;
        if let Some(keymap) = boot::keymap(&fleet)? {
            cmd::root("loadkeys", &[&keymap])?;
        }
        disk::prepare(&target, prompt)?;
    }

    let mut packages = vec!["base", "base-devel", "git", "chezmoi"];
    if net::remote(&url)?.is_some() {
        packages.push(Distro::Arch.ssh_package());
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
    carry_ssh(root, &root_user)?;
    let copy = file::under(root, fleet::DIR);
    if copy.exists() {
        pull_as_owner(&copy)?;
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

    let args = [disk::ROOT, BINARY, "install", ADMIN, &admin];
    let mut chroot = Command::new("arch-chroot");
    chroot.args(args).env(FROM_ISO, "1");
    cmd::status(chroot, "arch-chroot", &args)?;
    disk::close()?;
    println!(
        "The system is installed. Take out the USB stick and restart: type the disk's \
         passphrase, and install finishes there."
    );
    Ok(())
}

fn carry_ssh(root: &Path, root_user: &user::User) -> Result<()> {
    let known = Path::new(repo::KNOWN_HOSTS);
    if known.exists() {
        fs::copy(known, file::under(root, repo::KNOWN_HOSTS))?;
    }
    let Some(key) = file::read(&ssh::key_path(root_user))? else {
        return Ok(());
    };
    let carried = file::under(root, CARRIED_KEY);
    file::remove_if_found(&carried)?;
    fs::create_dir_all(carried.parent().expect("the carried key has a folder"))?;
    file::create_private(&carried, &key)
}

fn pull_as_owner(copy: &Path) -> Result<()> {
    let origin = checked::origin_of(Path::new(FLEET), || As::Root.command::<&str>("git", &[]))?;
    let meta = fs::metadata(copy)?;
    let (uid, gid) = (meta.uid().to_string(), meta.gid().to_string());
    let home = tempfile::tempdir()?;
    unix::fs::chown(home.path(), Some(meta.uid()), Some(meta.gid()))?;
    let as_owner = || {
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
        git
    };
    let theirs = checked::origin_of(copy, as_owner)?;
    if theirs != origin {
        bail!(
            "{} comes from {theirs}, not {origin}: run install with --fleet {theirs}",
            copy.display()
        );
    }
    let bundle = tempfile::Builder::new().suffix(".bundle").tempfile()?;
    let path = bundle.path().to_string_lossy().into_owned();
    cmd::root(
        "git",
        &["-C", FLEET, "bundle", "create", "--quiet", &path, "HEAD"],
    )?;
    fs::set_permissions(bundle.path(), fs::Permissions::from_mode(0o644))?;
    let args = [
        "-C",
        &copy.to_string_lossy(),
        "pull",
        "--quiet",
        "--ff-only",
        &path,
        "HEAD",
    ];
    let mut pull = as_owner();
    pull.args(args);
    cmd::status(pull, "git", &args)
}
