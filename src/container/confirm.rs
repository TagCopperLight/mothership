use super::{Pin, Service, registry::Client};
use crate::file;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

const PATH: &str = "/var/lib/mothership/unconfirmed.json";

#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
pub struct Unconfirmed(BTreeMap<String, Pin>);

impl Unconfirmed {
    pub fn load() -> Result<Self> {
        file::read_json(Path::new(PATH))
    }

    pub fn save(&self) -> Result<()> {
        file::write_json(Path::new(PATH), self)
    }

    pub fn note(&mut self, service: &str, pins: Vec<(String, Pin)>, by_versions_signer: bool) {
        for (image, pin) in pins {
            let key = key(service, &image);
            match by_versions_signer {
                true => drop(self.0.insert(key, pin)),
                false => drop(self.0.remove(&key)),
            }
        }
    }

    pub fn waits(&self, service: &str, image: &str, pin: &Pin) -> bool {
        self.0.get(&key(service, image)) == Some(pin)
    }

    pub fn confirm(&mut self, service: &str, image: &str) {
        self.0.remove(&key(service, image));
    }
}

fn key(service: &str, image: &str) -> String {
    format!("{service}/{image}")
}

pub fn ask_republished(
    type_name: &str,
    services: &[Service],
    registry: &Client,
    unconfirmed: &mut Unconfirmed,
    ask: &mut dyn FnMut(&str) -> Result<bool>,
) -> Result<()> {
    for service in services {
        let key = super::key(type_name, service);
        for (name, image) in &service.images {
            let pin = image.pin();
            if !unconfirmed.waits(&key, name, &pin) {
                continue;
            }
            let run = match registry.contradicts(image, &pin)? {
                None => true,
                Some(why) => {
                    println!("The probe pinned {} for {key}, and {why}.", pin.version);
                    ask("Run the pinned digest anyway")?
                }
            };
            if run {
                unconfirmed.confirm(&key, name);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/confirm.rs"]
mod tests;
