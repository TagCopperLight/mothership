mod boot;
mod disk;
mod iso;

use crate::{
    apply,
    cmd::{self, As},
    fleet::{self, Fleet, Machine},
    hardware, net,
    package::{self, Distro},
    prompt::Prompt,
    revert::Revert,
    scan,
    secrets::{self, Owner},
    user::{self, User},
};
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeMap,
    env, fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
    process::Stdio,
};

/// What install would otherwise ask, so cloud-init can run it with no questions.
#[derive(Default)]
struct Flags {
    fleet: Option<String>,
    device_type: Option<String>,
    options: Vec<String>,
    hostname: Option<String>,
    timezone: Option<String>,
    disk: Option<String>,
    layout: Option<disk::Layout>,
}

impl Flags {
    fn parse(args: &[String]) -> Result<Flags> {
        let mut flags = Flags::default();
        let mut args = args.iter();
        while let Some(flag) = args.next() {
            let value = args
                .next()
                .with_context(|| format!("{flag} needs a value"))?
                .clone();
            match flag.as_str() {
                "--fleet" => flags.fleet = Some(value),
                "--type" => flags.device_type = Some(value),
                "--option" => flags.options.push(value),
                "--hostname" => flags.hostname = Some(value),
                "--timezone" => flags.timezone = Some(value),
                "--disk" => flags.disk = Some(value),
                "--layout" => flags.layout = Some(value.parse()?),
                _ => bail!("install has no {flag}"),
            }
        }
        if !flags.options.is_empty() && flags.device_type.is_none() {
            bail!("--option needs --type");
        }
        Ok(flags)
    }
}

/// The Arch ISO creates it when it starts.
const ARCHISO: &str = "/run/archiso";
/// How the ISO hands its gh login to the new machine's admin.
const GH_TOKEN: &str = "MOTHERSHIP_GH_TOKEN";
/// Set by the ISO when it runs install inside the new system, before its first start.
const FROM_ISO: &str = "MOTHERSHIP_FROM_ISO";

pub fn run(args: &[String]) -> Result<()> {
    let flags = Flags::parse(args)?;
    let mut prompt = Prompt::new();
    if Path::new(ARCHISO).exists() {
        iso::run(&flags, &mut prompt)
    } else {
        here(&flags, &mut prompt)
    }
}

/// On the machine's own system: a server, the new disk from the ISO, or its first start.
fn here(flags: &Flags, prompt: &mut Prompt) -> Result<()> {
    let distro = Distro::here()?;
    let chroot = env::var_os(FROM_ISO).is_some();
    // An earlier run that was killed left it behind.
    if Path::new(NO_PASSWORD).exists() {
        fs::remove_file(NO_PASSWORD)?;
    }
    // The ISO hands over its gh login once, and no child sees it.
    let token = env::var(GH_TOKEN).ok();
    // SAFETY: install starts no thread before this.
    unsafe { env::remove_var(GH_TOKEN) };
    if !Path::new(fleet::MACHINE).exists() {
        start_server(flags, prompt, distro)?;
    }
    let machine = Machine::load()?;
    let dir = Path::new(fleet::DIR);
    let hardware = hardware::detect(Path::new("/"))?;
    let members = Fleet::new(
        dir.into(),
        &machine.device_type,
        &machine.options,
        &hardware,
    )?
    .members;
    for member in &members {
        if user::by_name(&member.name)?.is_none() {
            user::create(member)?;
        }
    }
    if fs::metadata(dir)?.uid() == 0 {
        let names: Vec<String> = members.iter().map(|m| m.name.clone()).collect();
        adopt(prompt, &names, dir, token.as_deref())?;
    }

    trust_keys(prompt, &hostname()?)?;
    let fleet = Fleet::load()?;
    let _sudo = match (distro, &fleet.system.aur_helper) {
        (Distro::Arch, Some(helper)) => Some(build_helper(fleet.admin(), helper)?),
        _ => None,
    };
    // A chroot runs no service manager, so the machine's first start does these.
    let skip = |member: Option<usize>, revert: &Revert| {
        chroot
            && member.is_some()
            && matches!(
                revert,
                Revert::Enable { .. } | Revert::Disable { .. } | Revert::Linger(_)
            )
    };
    net::retry("Applying the fleet", || apply::unasked(&skip))?;
    if distro == Distro::Debian {
        package::keep_only(&Fleet::load()?, prompt)?;
    }
    drop(_sudo);
    if chroot {
        return boot::setup();
    }
    if boot::first_start() {
        boot::finish()?;
    }
    scan::run()
}

/// A server comes with its system: it only needs the fleet and its answers.
fn start_server(flags: &Flags, prompt: &mut Prompt, distro: Distro) -> Result<()> {
    let admin = env::var("SUDO_USER")
        .ok()
        .and_then(|name| user::by_name(&name).transpose())
        .transpose()?
        .context("run install with sudo from the account that will commit the fleet")?;
    if !cmd::found(As::Root, "git") {
        net::retry("Installing git", || distro.bootstrap(&["git"]))?;
    }
    install_chezmoi(distro)?;
    let dir = Path::new(fleet::DIR);
    if !dir.join(".git").exists() {
        let url = match &flags.fleet {
            Some(url) => url.clone(),
            None => prompt.line("Fleet repository:")?,
        };
        admin.create_dir_all(dir)?;
        clone(&url, dir, As::User(&admin), distro)?;
    }
    let machine = ask_machine(flags, prompt, dir)?;
    let hostname = ask_hostname(flags, prompt)?;
    // The machine file comes last: once it's there, install skips these questions.
    cmd::root("hostnamectl", &["set-hostname", &hostname])?;
    machine.save(Path::new("/"))
}

fn hostname() -> Result<String> {
    Ok(fs::read_to_string("/etc/hostname")
        .context("reading /etc/hostname")?
        .trim()
        .to_owned())
}

fn ask_machine(flags: &Flags, prompt: &mut Prompt, dir: &Path) -> Result<Machine> {
    let types = fleet::types(dir)?;
    let device_type = match &flags.device_type {
        Some(name) if !types.contains(name) => bail!("the fleet has no type {name}"),
        Some(name) => name.clone(),
        None => types[prompt.choose("Device type", &types)?].clone(),
    };
    let hardware = hardware::detect(Path::new("/"))?;
    let offered = Fleet::new(dir.into(), &device_type, &[], &hardware)?.options();
    let options = match &flags.device_type {
        Some(_) => {
            if let Some(unknown) = flags.options.iter().find(|o| !offered.contains(o)) {
                bail!("{device_type} has no option {unknown}");
            }
            flags.options.clone()
        }
        None => {
            let mut options = Vec::new();
            for option in offered {
                if prompt.confirm(&format!("Turn on {option}"))? {
                    options.push(option);
                }
            }
            options
        }
    };
    Ok(Machine {
        device_type,
        options,
    })
}

fn ask_hostname(flags: &Flags, prompt: &mut Prompt) -> Result<String> {
    let valid = |name: &str| {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    match &flags.hostname {
        Some(name) if valid(name) => Ok(name.clone()),
        Some(name) => bail!("{name} isn't a hostname: use a-z, 0-9 and -"),
        None => loop {
            let name = prompt.line("Hostname:")?;
            if valid(&name) {
                return Ok(name);
            }
            println!("Use a-z, 0-9 and -.");
        },
    }
}

/// Clones with gh's login when the repository is private. Returns whether it logged in.
fn clone(url: &str, dir: &Path, who: As, distro: Distro) -> Result<bool> {
    let path = dir.to_string_lossy();
    // git fails the same way for a private repository and a dropped network, so it only
    // counts as private once the network works.
    let public = net::retry("Reaching the fleet", || {
        let mut probe = who.command("git", &["ls-remote", url]);
        probe
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        match probe.status().context("running git")?.success() {
            true => Ok(()),
            false => bail!("git can't read {url} without a login"),
        }
    })
    .is_ok();
    let empty = || -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
        }
        Ok(())
    };
    if public {
        net::retry("Cloning the fleet", || {
            empty()?;
            cmd::run(who, "git", &["clone", url, &path])
        })?;
        return Ok(false);
    }
    if !cmd::found(As::Root, "gh") {
        let name = match distro {
            Distro::Arch => "github-cli",
            Distro::Debian => "gh",
        };
        net::retry("Installing gh", || distro.bootstrap(&[name]))?;
    }
    if cmd::bytes(who, "gh", &["auth", "status"]).is_err() {
        println!("The fleet is private: log in to GitHub.");
        let args = ["auth", "login", "--hostname", "github.com"];
        cmd::run(
            who,
            "gh",
            &[&args[..], &["--git-protocol", "https", "--web"]].concat(),
        )?;
    }
    net::retry("Cloning the fleet", || {
        empty()?;
        cmd::run(who, "gh", &["repo", "clone", url, &path])
    })?;
    Ok(true)
}

/// git as the admin, with gh's login: the admin's own git config comes later, from chezmoi.
fn git(admin: &User, args: &[&str]) -> Result<()> {
    let helper = [
        "-c",
        "credential.helper=",
        "-c",
        "credential.helper=!gh auth git-credential",
    ];
    cmd::run(As::User(admin), "git", &[&helper[..], args].concat())
}

fn install_chezmoi(distro: Distro) -> Result<()> {
    if cmd::found(As::Root, "chezmoi") {
        return Ok(());
    }
    if distro == Distro::Arch {
        return net::retry("Installing chezmoi", || distro.bootstrap(&["chezmoi"]));
    }
    // Debian has no package for it: the .deb comes from chezmoi's GitHub releases.
    let arch = cmd::stdout(As::Root, "dpkg", &["--print-architecture"])?;
    let suffix = format!("_linux_{}.deb", arch.trim());
    net::retry("Installing chezmoi", || {
        let latest = "https://api.github.com/repos/twpayne/chezmoi/releases/latest";
        let release: serde_json::Value =
            serde_json::from_str(&cmd::stdout(As::Root, "curl", &["-fsSL", latest])?)?;
        let asset = |wanted: &dyn Fn(&str) -> bool| -> Result<(String, String)> {
            release["assets"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|asset| asset["name"].as_str().is_some_and(wanted))
                .and_then(|asset| {
                    let name = asset["name"].as_str()?.to_owned();
                    Some((name, asset["browser_download_url"].as_str()?.to_owned()))
                })
                .context("chezmoi's latest release lacks a file install needs")
        };
        let (name, url) = asset(&|n| n.ends_with(&suffix))?;
        let (_, sums_url) = asset(&|n| n.ends_with("_checksums.txt"))?;
        let dir = tempfile::tempdir()?;
        let deb = dir.path().join(&name).to_string_lossy().into_owned();
        cmd::root("curl", &["-fsSL", "-o", &deb, &url])?;
        let sums = cmd::stdout(As::Root, "curl", &["-fsSL", &sums_url])?;
        let wanted = sums
            .lines()
            .find_map(|line| line.strip_suffix(name.as_str())?.split_whitespace().next())
            .with_context(|| format!("chezmoi's checksums don't list {name}"))?;
        let actual = cmd::stdout(As::Root, "sha256sum", &[&deb])?;
        if actual.split_whitespace().next() != Some(wanted) {
            bail!("{name} doesn't match chezmoi's checksum");
        }
        distro.bootstrap(&[&deb])
    })
}

/// The fleet the ISO copied belongs to root: one of the type's users becomes its admin.
fn adopt(prompt: &mut Prompt, names: &[String], dir: &Path, token: Option<&str>) -> Result<()> {
    if names.is_empty() {
        bail!("the type lists no user to own the fleet");
    }
    let name = &names[prompt.choose("Admin, who commits and pushes the fleet", names)?];
    let admin = user::by_name(name)?.context("created above")?;
    println!("{name}'s password:");
    let mut tries = 0;
    while let Err(e) = cmd::root("passwd", &[name]) {
        tries += 1;
        if tries == 3 {
            return Err(e);
        }
    }
    admin.own_all(dir)?;
    if let Some(token) = token {
        let args = ["auth", "login", "--hostname", "github.com", "--with-token"];
        cmd::with_input(As::User(&admin), "gh", &args, token.as_bytes())?;
    }
    Ok(())
}

const NO_PASSWORD: &str = "/etc/sudoers.d/zz-mothership-install";

/// The AUR helper builds as the admin and calls sudo, so the admin gets sudo without a
/// password while install applies the fleet.
struct NoPassword(libc::sighandler_t);

/// Ctrl-C stops the program running, whose failure then unwinds install and removes the file.
extern "C" fn on_interrupt(_: libc::c_int) {}

impl NoPassword {
    fn grant(admin: &User) -> Result<NoPassword> {
        // SAFETY: the handler does nothing, and Drop puts the old one back.
        let handler = on_interrupt as extern "C" fn(libc::c_int) as libc::sighandler_t;
        let granted = NoPassword(unsafe { libc::signal(libc::SIGINT, handler) });
        let rule = format!("{} ALL=(ALL:ALL) NOPASSWD: ALL\n", admin.name);
        fs::write(NO_PASSWORD, rule)?;
        fs::set_permissions(NO_PASSWORD, fs::Permissions::from_mode(0o440))?;
        Ok(granted)
    }
}

impl Drop for NoPassword {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_file(NO_PASSWORD) {
            eprintln!("Remove {NO_PASSWORD} by hand: {e}");
        }
        // SAFETY: puts back the handler grant replaced.
        unsafe { libc::signal(libc::SIGINT, self.0) };
    }
}

/// The helper is an AUR package itself, so a new machine builds it with makepkg first.
fn build_helper(admin: &User, helper: &str) -> Result<NoPassword> {
    let granted = NoPassword::grant(admin)?;
    if cmd::found(As::User(admin), helper) {
        return Ok(granted);
    }
    let build = tempfile::tempdir()?;
    admin.own(build.path())?;
    let dir = build.path().join(helper).to_string_lossy().into_owned();
    let url = format!("https://aur.archlinux.org/{helper}.git");
    net::retry("Building the AUR helper", || {
        if Path::new(&dir).exists() {
            fs::remove_dir_all(&dir)?;
        }
        cmd::run(As::User(admin), "git", &["clone", &url, &dir])?;
        let make = "cd \"$1\" && makepkg --syncdeps --install --rmdeps --noconfirm";
        cmd::run(As::User(admin), "sh", &["-c", make, "sh", &dir])
    })?;
    Ok(granted)
}

/// A list's machines run `trust` for the new keys, or I type the recovery key here.
fn trust_keys(prompt: &mut Prompt, hostname: &str) -> Result<()> {
    loop {
        let fleet = Fleet::load()?;
        let mut waiting = Vec::new();
        for owner in secrets::keys(&fleet)? {
            if owner.trusted {
                continue;
            }
            if secrets::names(&owner.list)?.is_empty() {
                secrets::trust(&fleet, &owner.target, hostname, &owner.public)?;
            } else {
                waiting.push(owner);
            }
        }
        if waiting.is_empty() {
            return fleet.hand_back();
        }
        let (question, keys): (&str, &[char]) = match print_trust(&waiting, hostname)? {
            true => (
                "d done, pull the fleet · r type the recovery key ›",
                &['d', 'r'],
            ),
            false => ("r type the recovery key ›", &['r']),
        };
        match prompt.ask(question, keys)? {
            Some('d') => net::retry("Pulling the fleet", || {
                git(fleet.admin(), &["-C", fleet::DIR, "pull", "--ff-only"])
            })?,
            Some('r') => {
                let mut keys = fleet.keys().clone();
                let public = keys.add(&prompt.hidden("Recovery key:")?)?;
                for owner in &waiting {
                    if !secrets::holds(&owner.list, &public)? {
                        bail!("that key isn't in {}", owner.list.display());
                    }
                }
                for owner in &waiting {
                    secrets::trust_with(&fleet, &keys, &owner.target, hostname, &owner.public)?;
                }
                fleet.hand_back()?;
            }
            _ => bail!("install stopped before the fleet trusted this machine's keys"),
        }
    }
}

/// Returns whether any machine opens the lists, which leaves only the recovery key otherwise.
fn print_trust(waiting: &[Owner], hostname: &str) -> Result<bool> {
    let mut by_machine: BTreeMap<String, Vec<&Owner>> = BTreeMap::new();
    for owner in waiting {
        for name in secrets::names(&owner.list)? {
            if name != secrets::RECOVERY {
                by_machine.entry(name).or_default().push(owner);
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
    for (machine, owners) in by_machine {
        println!("  On {machine}:");
        for owner in owners {
            println!(
                "    sudo mothership trust {} {hostname} {}",
                owner.target, owner.public
            );
        }
    }
    Ok(true)
}
