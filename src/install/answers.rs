use super::disk::Layout;
use crate::{
    fleet::{self, Fleet, Machine},
    hardware,
    prompt::{Choice, Prompt},
};
use anyhow::{Context, Result, bail};
use std::path::{Component, Path, PathBuf};

#[derive(Default)]
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
            let value = args
                .next()
                .with_context(|| format!("{flag} needs a value"))?
                .clone();
            match flag.as_str() {
                "--fleet" => flags.fleet = Some(value),
                "--type" => flags.device_type = Some(value),
                "--option" => flags.options.push(value),
                "--hostname" => flags.hostname = Some(value),
                ADMIN => flags.admin = Some(value),
                "--remove-unlisted" => flags.remove_unlisted = Some(yes_or_no(flag, &value)?),
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

pub fn machine(flags: &Flags, prompt: &mut Prompt, dir: &Path) -> Result<Machine> {
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

fn valid_hostname(name: &str) -> bool {
    (1..=63).contains(&name.len())
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn hostname(flags: &Flags, prompt: &mut Prompt) -> Result<String> {
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
#[path = "../../tests/install.rs"]
mod tests;
