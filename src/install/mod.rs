mod boot;
mod disk;
mod iso;

use crate::{
    apply, chezmoi,
    cmd::{self, As},
    fleet::{self, Fleet, Machine},
    hardware, net,
    package::{self, Distro},
    prompt::{Choice, Prompt},
    scan,
    secrets::{self, MachineKey},
    user::{self, User},
    walk,
};
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeMap, env, ffi::CStr, fs, os::unix::fs::MetadataExt, path::Path,
    process::Stdio,
};

#[derive(Default)]
struct Flags {
    fleet: Option<String>,
    device_type: Option<String>,
    options: Vec<String>,
    hostname: Option<String>,
    admin: Option<String>,
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
                "--admin" => flags.admin = Some(value),
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

    fn only_on_the_iso(&self) -> Option<&'static str> {
        [
            (self.timezone.is_some(), "--timezone"),
            (self.disk.is_some(), "--disk"),
            (self.layout.is_some(), "--layout"),
        ]
        .into_iter()
        .find_map(|(given, flag)| given.then_some(flag))
    }
}

const ARCHISO: &str = "/run/archiso";
const GH_TOKEN: &str = "MOTHERSHIP_GH_TOKEN";
const FROM_ISO: &str = "MOTHERSHIP_FROM_ISO";
const BINARY: &str = "/usr/local/bin/mothership";
const GH_LOGIN: [&str; 4] = ["auth", "login", "--hostname", "github.com"];

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
    let no_password = no_password_path();
    if no_password.exists() {
        fs::remove_file(no_password)?;
    }
    let token = env::var(GH_TOKEN).ok();
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
        let names: Vec<Choice> = members.iter().map(|m| Choice::new(&m.name, "")).collect();
        adopt(prompt, &names, dir, token.as_deref())?;
    }

    let first_start = !chroot && boot::first_start();
    if first_start {
        boot::secure()?;
    }
    trust_keys(prompt, &hostname()?)?;
    let fleet = Fleet::load()?;
    let sudo = match (distro, &fleet.system.aur_helper) {
        (Distro::Arch, Some(helper)) => Some(build_helper(fleet.admin(), helper)?),
        _ => None,
    };
    net::retry("Applying the fleet", || apply::unasked(!chroot))?;
    drop(sudo);
    if distro == Distro::Debian {
        package::keep_only(&Fleet::load()?, prompt)?;
    }
    if chroot {
        return boot::setup();
    }
    if first_start {
        boot::finish()?;
    }
    scan::run()
}

fn start_server(flags: &Flags, prompt: &mut Prompt, distro: Distro) -> Result<()> {
    let name = match &flags.admin {
        Some(name) => name.clone(),
        None => env::var("SUDO_USER").context(
            "name the account that will commit the fleet with --admin, or run install with sudo \
             from it",
        )?,
    };
    let admin = user::by_name(&name)?.with_context(|| format!("there is no user {name}"))?;
    if !cmd::found(As::Root, "git") {
        net::retry("Installing git", || distro.bootstrap(&["git"]))?;
    }
    if !cmd::found(As::Root, "chezmoi") {
        net::retry("Installing chezmoi", || match distro {
            Distro::Arch => distro.bootstrap(&["chezmoi"]),
            Distro::Debian => {
                let dir = tempfile::tempdir()?;
                let deb = chezmoi::download_deb(dir.path())?;
                distro.bootstrap(&[&deb.to_string_lossy()])
            }
        })?;
    }
    let dir = Path::new(fleet::DIR);
    if !dir.join(".git").exists() {
        let url = match &flags.fleet {
            Some(url) => url.clone(),
            None => prompt.line("Fleet repository:")?,
        };
        fs::create_dir_all(dir.parent().expect("the fleet folder has a parent"))?;
        admin.create_dir_all(dir)?;
        clone(&url, dir, As::User(&admin), distro)?;
    }
    let machine = ask_machine(flags, prompt, dir)?;
    let hostname = ask_hostname(flags, prompt)?;
    set_hostname(&hostname)?;
    machine.save(Path::new("/"))
}

fn set_hostname(new: &str) -> Result<()> {
    let old = hostname()?;
    cmd::root("hostnamectl", &["set-hostname", new])?;
    let hosts = fs::read_to_string("/etc/hosts").context("reading /etc/hosts")?;
    let renamed: Vec<String> = hosts
        .lines()
        .map(|line| match line.strip_prefix("127.0.1.1") {
            Some(names) => {
                let names: Vec<&str> = names
                    .split_whitespace()
                    .map(|name| if name == old { new } else { name })
                    .collect();
                format!("127.0.1.1\t{}", names.join(" "))
            }
            None => line.to_owned(),
        })
        .collect();
    if renamed.join("\n") != hosts.trim_end_matches('\n') {
        fs::write("/etc/hosts", renamed.join("\n") + "\n")?;
    }
    Ok(())
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
        None => {
            let choices: Vec<Choice> = types.iter().map(|t| Choice::new(t, "")).collect();
            types[prompt.choose("Device type", &choices)?].clone()
        }
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

fn valid_hostname(name: &str) -> bool {
    (1..=63).contains(&name.len())
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn ask_hostname(flags: &Flags, prompt: &mut Prompt) -> Result<String> {
    const RULE: &str = "use up to 63 of a-z, 0-9 and -, with no - at either end";
    match &flags.hostname {
        Some(name) if valid_hostname(name) => Ok(name.clone()),
        Some(name) => bail!("{name} isn't a hostname: {RULE}"),
        None => loop {
            let name = prompt.line("Hostname:")?;
            if valid_hostname(&name) {
                return Ok(name);
            }
            println!("That isn't a hostname: {RULE}.");
        },
    }
}

fn clone(url: &str, dir: &Path, who: As, distro: Distro) -> Result<bool> {
    let path = dir.to_string_lossy();
    let readable = net::retry("Reaching the fleet", || {
        let mut probe = who.command("git", &["ls-remote", url]);
        probe
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        match probe.status().context("running git")?.success() {
            true => Ok(()),
            false => bail!("git can't read {url}"),
        }
    });
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
    let private = match readable {
        Ok(()) => false,
        Err(_) if url.starts_with("https://github.com/") => true,
        Err(e) => return Err(e.context("only a GitHub repository can use a login")),
    };
    if !private {
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
    if cmd::bytes(who, "gh", &["auth", "token"]).is_err() {
        println!("The fleet is private: log in to GitHub.");
        let args = [&GH_LOGIN[..], &["--git-protocol", "https", "--web"]].concat();
        cmd::run(who, "gh", &args)?;
    }
    net::retry("Cloning the fleet", || {
        empty()?;
        cmd::run(who, "gh", &["repo", "clone", url, &path])
    })?;
    Ok(true)
}

fn git(who: As, args: &[&str]) -> Result<()> {
    let helper = [
        "-c",
        "credential.helper=",
        "-c",
        "credential.helper=!gh auth git-credential",
    ];
    cmd::run(who, "git", &[&helper[..], args].concat())
}

fn adopt(prompt: &mut Prompt, names: &[Choice], dir: &Path, token: Option<&str>) -> Result<()> {
    let name = &names[prompt.choose("Admin, who commits and pushes the fleet", names)?].name;
    let admin = user::by_name(name)?.context("install creates the type's users first")?;
    println!("{name}'s password:");
    let mut tries = 0;
    while let Err(e) = cmd::root("passwd", &[name]) {
        tries += 1;
        if tries == 3 {
            return Err(e);
        }
    }
    if let Some(token) = token {
        let args = [&GH_LOGIN[..], &["--with-token"]].concat();
        net::retry("Logging in to GitHub", || {
            cmd::with_input(As::User(&admin), "gh", &args, token.as_bytes())
        })?;
    }
    admin.own_all(dir)
}

const NO_PASSWORD: &CStr = c"/etc/sudoers.d/zz-mothership-install";
const LEAVING: [libc::c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

fn no_password_path() -> &'static Path {
    Path::new(NO_PASSWORD.to_str().expect("the path is ASCII"))
}

struct NoPassword([libc::sighandler_t; 3]);

extern "C" fn remove_and_leave(signal: libc::c_int) {
    unsafe {
        libc::unlink(NO_PASSWORD.as_ptr());
        libc::signal(signal, libc::SIG_DFL);
        libc::raise(signal);
    }
}

impl NoPassword {
    fn grant(admin: &User) -> Result<NoPassword> {
        let handler = remove_and_leave as extern "C" fn(libc::c_int) as libc::sighandler_t;
        let previous = LEAVING.map(|signal| unsafe { libc::signal(signal, handler) });
        let granted = NoPassword(previous);
        let rule = format!("{} ALL=(root) NOPASSWD: /usr/bin/pacman\n", admin.name);
        walk::write(no_password_path(), rule.as_bytes(), 0o440)?;
        Ok(granted)
    }
}

impl Drop for NoPassword {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_file(no_password_path()) {
            eprintln!("Remove {} by hand: {e}", no_password_path().display());
        }
        for (signal, previous) in LEAVING.into_iter().zip(self.0) {
            unsafe { libc::signal(signal, previous) };
        }
    }
}

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

fn trust_keys(prompt: &mut Prompt, hostname: &str) -> Result<()> {
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
        let (question, answers): (&str, &[char]) = match print_trust(&waiting, hostname)? {
            true => (
                "d done, pull the fleet · r type the recovery key ›",
                &['d', 'r'],
            ),
            false => ("r type the recovery key ›", &['r']),
        };
        match prompt.ask(question, answers)? {
            Some('d') => net::retry("Pulling the fleet", || {
                git(
                    As::User(fleet.admin()),
                    &["-C", fleet::DIR, "pull", "--ff-only"],
                )
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

fn print_trust(waiting: &[MachineKey], hostname: &str) -> Result<bool> {
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

#[cfg(test)]
#[path = "../../tests/install.rs"]
mod tests;
