use crate::{
    container::{SECRETS, SERVICE_TOML, SERVICES},
    file,
    fleet::{self, Fleet, homes},
    host,
    prompt::Prompt,
    secrets::{self, AGE},
    trust,
};
use anyhow::{Result, bail};
use std::{fs, path::PathBuf};

pub struct Wanted {
    pub type_name: String,
    pub service: String,
    pub name: String,
}

impl Wanted {
    pub fn parse(service: &str, name: &str, own_type: &str) -> Result<Wanted> {
        let (type_name, service) = service.split_once('/').unwrap_or((own_type, service));
        for part in [type_name, service, name] {
            host::check(part)?;
        }
        Ok(Wanted {
            type_name: type_name.to_owned(),
            service: service.to_owned(),
            name: name.to_owned(),
        })
    }

    pub fn shown(&self) -> String {
        format!("{}/{}", self.type_name, self.service)
    }
}

pub fn set(fleet: &Fleet, wanted: &Wanted, prompt: &mut Prompt) -> Result<()> {
    let dir = fleet::type_dir(&fleet.dir, &wanted.type_name)
        .join(SERVICES)
        .join(&wanted.service);
    if !dir.join(SERVICE_TOML).is_file() {
        bail!("the fleet has no service {}", wanted.shown());
    }
    let path = dir.join(SECRETS).join(format!("{}{AGE}", wanted.name));
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
#[path = "../../tests/service_secret.rs"]
mod tests;
