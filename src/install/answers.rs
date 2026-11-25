use super::disk::Layout;
use crate::{
    fleet::{
        self, Machine, Repo,
        machines::{self, Entry, Machines},
    },
    system::host,
    util::file,
    util::prompt::{Choice, Prompt},
};
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

const SAVED: &str = "/tmp/mothership-answers.toml";

#[derive(Default, Clone)]
pub struct Flags {
    pub fleet: Option<String>,
    pub device_type: Option<String>,
    pub options: Vec<String>,
    pub hostname: Option<String>,
    pub admin: Option<String>,
    pub remove_unlisted: Option<bool>,
    pub timezone: Option<String>,
    pub disk: Option<String>,
    pub layout: Option<Layout>,
}

pub const ADMIN: &str = "--admin";

impl Flags {
    pub fn parse(args: &[String]) -> Result<Flags> {
        let mut flags = Flags::default();
        let mut args = args.iter();
        while let Some(flag) = args.next() {
            let value = match args.next() {
                Some(value) if !value.starts_with("--") => value.clone(),
                _ => bail!("{flag} needs a value"),
            };
            match flag.as_str() {
                "--fleet" => once(&mut flags.fleet, flag, value)?,
                "--type" => once(&mut flags.device_type, flag, value)?,
                "--option" if flags.options.contains(&value) => {
                    bail!("{flag} {value} is given twice")
                }
                "--option" => flags.options.push(value),
                "--hostname" => once(&mut flags.hostname, flag, value)?,
                ADMIN => once(&mut flags.admin, flag, value)?,
                "--remove-unlisted" => {
                    once(&mut flags.remove_unlisted, flag, yes_or_no(flag, &value)?)?
                }
                "--timezone" => once(&mut flags.timezone, flag, value)?,
                "--disk" => once(&mut flags.disk, flag, value)?,
                "--layout" => {
                    let layout = value
                        .parse()
                        .map_err(|_| anyhow!("the layout is wipe or beside, not {value}"))?;
                    once(&mut flags.layout, flag, layout)?
                }
                _ => bail!("install has no {flag}"),
            }
        }
        if !flags.options.is_empty() && flags.device_type.is_none() {
            bail!("--option needs --type");
        }
        Ok(flags)
    }

    pub fn only_on_the_iso(&self) -> Option<&'static str> {
        [
            (self.timezone.is_some(), "--timezone"),
            (self.disk.is_some(), "--disk"),
            (self.layout.is_some(), "--layout"),
        ]
        .into_iter()
        .find_map(|(given, flag)| given.then_some(flag))
    }
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Saved {
    pub fleet: String,
    pub hostname: String,
    pub admin: String,
    pub timezone: String,
    pub machine: Machine,
}

impl Saved {
    pub fn offer(prompt: &mut Prompt) -> Result<Option<Saved>> {
        let Some(text) = file::read_if_any(Path::new(SAVED))? else {
            return Ok(None);
        };
        let saved: Saved = toml::from_str(&text).with_context(|| format!("parsing {SAVED}"))?;
        println!("The last run's answers:");
        println!("  {}", saved.describe());
        Ok(prompt.confirm("Use these answers")?.then_some(saved))
    }

    pub fn save(&self) -> Result<()> {
        file::write(Path::new(SAVED), toml::to_string(self)?.as_bytes(), 0o600)
    }

    pub fn forget() -> Result<()> {
        file::remove_if_found(Path::new(SAVED)).map(drop)
    }

    fn describe(&self) -> String {
        let options = match self.machine.options.is_empty() {
            true => String::new(),
            false => format!(" with {}", self.machine.options.join(", ")),
        };
        format!(
            "{} from {}, {}{options}, admin {}, time zone {}",
            self.hostname, self.fleet, self.machine.device_type, self.admin, self.timezone
        )
    }

    pub fn under(self, flags: &Flags) -> Flags {
        let mut filled = flags.clone();
        filled.fleet.get_or_insert(self.fleet);
        filled.hostname.get_or_insert(self.hostname);
        filled.admin.get_or_insert(self.admin);
        filled.timezone.get_or_insert(self.timezone);
        if filled.device_type.is_none() {
            filled.device_type = Some(self.machine.device_type);
            filled.options = self.machine.options;
        }
        filled
    }
}

fn once<T>(slot: &mut Option<T>, flag: &str, value: T) -> Result<()> {
    if slot.is_some() {
        bail!("{flag} is given twice");
    }
    *slot = Some(value);
    Ok(())
}

fn yes_or_no(flag: &str, value: &str) -> Result<bool> {
    match value {
        "yes" => Ok(true),
        "no" => Ok(false),
        _ => bail!("{flag} is yes or no, not {value}"),
    }
}

pub fn fleet_url(flags: &Flags, prompt: &mut Prompt) -> Result<String> {
    match &flags.fleet {
        Some(url) => Ok(url.clone()),
        None => prompt.line("Fleet repository:"),
    }
}

pub fn machine(flags: &Flags, prompt: &mut Prompt, dir: &Path, hostname: &str) -> Result<Machine> {
    match Machines::load(dir)?.get(hostname) {
        Some(entry) => listed(flags, entry, dir, hostname),
        None => asked(flags, prompt, dir),
    }
}

fn listed(flags: &Flags, entry: &Entry, dir: &Path, hostname: &str) -> Result<Machine> {
    let file = machines::path(Path::new(""), hostname);
    if let Some(given) = &flags.device_type
        && given != &entry.device_type
    {
        bail!(
            "--type {given}: {} says {hostname} is a {}",
            file.display(),
            entry.device_type
        );
    }
    let mut given = flags.options.clone();
    let mut listed = entry.options.clone();
    given.sort();
    listed.sort();
    if flags.device_type.is_some() && given != listed {
        bail!(
            "--option: {} lists other options for {hostname}",
            file.display()
        );
    }
    Machines::check_options(dir, entry)?;
    Ok(Machine {
        device_type: entry.device_type.clone(),
        options: entry.options.clone(),
    })
}

fn asked(flags: &Flags, prompt: &mut Prompt, dir: &Path) -> Result<Machine> {
    let types = fleet::types(dir)?;
    let device_type = match &flags.device_type {
        Some(name) if !types.contains(name) => bail!("the fleet has no type {name}"),
        Some(name) => name.clone(),
        None => {
            let choices: Vec<Choice> = types.iter().map(|t| Choice::new(t, "")).collect();
            types[prompt.choose("Device type", &choices)?].clone()
        }
    };
    let mut machine = Machine {
        device_type,
        options: Vec::new(),
    };
    let offered = Repo::for_machine(dir.into(), &machine)?.options();
    let device_type = &machine.device_type;
    machine.options = match &flags.device_type {
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
    Ok(machine)
}

pub fn admin_name(flags: &Flags, prompt: &mut Prompt) -> Result<String> {
    match &flags.admin {
        Some(name) => Ok(name.clone()),
        None => prompt.line("Admin, who commits and pushes the fleet:"),
    }
}

pub fn check_admin(dir: &Path, machine: &Machine, admin: &str) -> Result<()> {
    let fleet = Repo::for_machine(dir.into(), machine)?;
    if !fleet.profiles.iter().any(|member| member.name == admin) {
        bail!("{} has no user {admin}", machine.device_type);
    }
    Ok(())
}

pub fn admin_given(flags: &Flags, members: &[String]) -> Result<Option<String>> {
    match &flags.admin {
        Some(name) if !members.contains(name) => {
            bail!("{ADMIN} {name}: the type has no user {name}")
        }
        given => Ok(given.clone()),
    }
}

pub fn admin(flags: &Flags, prompt: &mut Prompt, members: &[String]) -> Result<String> {
    if let Some(name) = admin_given(flags, members)? {
        return Ok(name);
    }
    let choices: Vec<Choice> = members.iter().map(|m| Choice::new(m, "")).collect();
    Ok(members[prompt.choose("Admin, who commits and pushes the fleet", &choices)?].clone())
}

pub fn hostname(flags: &Flags, prompt: &mut Prompt) -> Result<String> {
    match &flags.hostname {
        Some(name) => host::check(name).map(|()| name.clone()),
        None => loop {
            let name = prompt.line("Hostname:")?;
            match host::check(&name) {
                Ok(()) => return Ok(name),
                Err(e) => println!("{e}."),
            }
        },
    }
}

pub fn zoneinfo(timezone: &str) -> PathBuf {
    Path::new("/usr/share/zoneinfo").join(timezone)
}

fn known_timezone(name: &str) -> bool {
    Path::new(name)
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
        && zoneinfo(name).is_file()
}

pub fn timezone(flags: &Flags, prompt: &mut Prompt) -> Result<String> {
    match &flags.timezone {
        Some(name) if known_timezone(name) => Ok(name.clone()),
        Some(name) => bail!("{name} isn't a time zone, such as Europe/Paris"),
        None => loop {
            let name = prompt.line("Time zone, such as Europe/Paris:")?;
            if known_timezone(&name) {
                return Ok(name);
            }
            println!("{name} isn't in /usr/share/zoneinfo.");
        },
    }
}

#[cfg(test)]
#[path = "../../tests/install/answers.rs"]
mod tests;
