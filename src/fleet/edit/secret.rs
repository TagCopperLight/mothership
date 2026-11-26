use crate::{
    fleet::{self, Fleet, Here, homes, machines::Machines, sync},
    secrets::{self, AGE},
    services::{SECRETS, SERVICE_TOML, SERVICES, TOKEN},
    system::host,
    trust,
    util::file,
    util::prompt::Prompt,
};
use anyhow::{Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Wanted {
    type_name: String,
    service: Option<String>,
    name: String,
}

impl Wanted {
    fn parse(service: &str, name: &str, own_type: &str) -> Result<Wanted> {
        let (type_name, service) = service.split_once('/').unwrap_or((own_type, service));
        for part in [type_name, service, name] {
            host::check(part)?;
        }
        Ok(Wanted {
            type_name: type_name.to_owned(),
            service: Some(service.to_owned()),
            name: name.to_owned(),
        })
    }

    fn token(wanted: &str, own_type: &str) -> Result<Wanted> {
        let (type_name, name) = wanted.split_once('/').unwrap_or((own_type, wanted));
        let token = TOKEN.trim_end_matches(AGE);
        if name != token {
            bail!("a secret names its service, unless it's [<type>/]{token}");
        }
        host::check(type_name)?;
        Ok(Wanted {
            type_name: type_name.to_owned(),
            service: None,
            name: name.to_owned(),
        })
    }

    fn shown(&self) -> String {
        match &self.service {
            Some(service) => format!("{}/{service}", self.type_name),
            None => format!("the {} services", self.type_name),
        }
    }

    fn path(&self, fleet: &Fleet) -> Result<PathBuf> {
        let dir = fleet::type_dir(&fleet.dir, &self.type_name).join(SERVICES);
        let Some(service) = &self.service else {
            return Ok(dir.join(TOKEN));
        };
        let dir = dir.join(service);
        if !dir.join(SERVICE_TOML).is_file() {
            bail!("the fleet has no service {}", self.shown());
        }
        Ok(dir.join(SECRETS).join(format!("{}{AGE}", self.name)))
    }
}

pub fn run(service: &str, name: Option<&str>) -> Result<()> {
    let dir = Path::new(sync::CURRENT);
    let machine = Here::load()?.machine(&Machines::load(dir)?)?;
    let wanted = match name {
        Some(name) => Wanted::parse(service, name, &machine.device_type)?,
        None => Wanted::token(service, &machine.device_type)?,
    };
    let message = format!("Set {} of {}", wanted.name, wanted.shown());
    trust::commit::edit_fleet(&message, |fleet, prompt| set(fleet, &wanted, prompt)).map(drop)
}

fn set(fleet: &Fleet, wanted: &Wanted, prompt: &mut Prompt) -> Result<()> {
    let path = wanted.path(fleet)?;
    let value = prompt.hidden(&format!("{}'s value", wanted.name))?;
    if value.is_empty() {
        bail!("{} stays as it was, since the value is empty", wanted.name);
    }
    let file: PathBuf = match fs::symlink_metadata(&path).is_ok_and(|m| m.is_symlink()) {
        true => {
            trust::Readers::new(prompt).approve(fleet, &path)?;
            homes::target(&fleet.dir, &path)?
        }
        false => path.clone(),
    };
    let sealed = secrets::encrypt_for(&trust::sealed_list(fleet, &path)?, value.as_bytes())?;
    file::write(&file, &sealed, 0o644)
}

#[cfg(test)]
#[path = "../../../tests/fleet/edit/secret.rs"]
mod tests;
