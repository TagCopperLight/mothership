use super::{Image, Pin, Service, changed_pins, recipe, registry::Client, service_of};
use crate::{
    fleet::{
        TYPES,
        git::{GIT_DIR, Git},
        sync,
    },
    trust::sign::{Reach, Signer},
    trust::verify::{self, Step},
    util::file,
};
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

pub fn note_inherited(git: &Git, anchor: &str) -> Result<()> {
    let mut unconfirmed = Unconfirmed::load()?;
    inherited(git, anchor, &mut unconfirmed)?;
    unconfirmed.save()
}

fn inherited(git: &Git, anchor: &str, unconfirmed: &mut Unconfirmed) -> Result<()> {
    for path in git.files(anchor, TYPES)? {
        let Some(service) = service_of(&path) else {
            continue;
        };
        let path = path.to_str().expect("a service file's path is UTF-8");
        let mut open = changed_pins(None, git.blob(anchor, path)?.as_deref());
        let history = git.out(&["rev-list", "--first-parent", anchor, "--", path])?;
        for commit in history.lines() {
            if open.is_empty() {
                break;
            }
            let parent = git.rev(&format!("{commit}^1"))?;
            let old = match &parent {
                Some(parent) => git.blob(parent, path)?,
                None => None,
            };
            let set = changed_pins(old.as_deref(), git.blob(commit, path)?.as_deref());
            let by_versions = match &parent {
                Some(parent) => !matches!(
                    verify::verify(git, parent, commit),
                    Ok(Some(Signer {
                        reach: Reach::Everything | Reach::Folders(_),
                        ..
                    })) | Ok(None)
                ),
                None => false,
            };
            let (now, later): (Vec<_>, Vec<_>) =
                open.into_iter().partition(|pin| set.contains(pin));
            unconfirmed.note(&service, now, by_versions);
            open = later;
        }
        unconfirmed.note(&service, open, true);
    }
    Ok(())
}

pub fn note_unconfirmed(git: &Git, steps: &[Step]) -> Result<()> {
    if steps.is_empty() {
        return Ok(());
    }
    let mut unconfirmed = Unconfirmed::load()?;
    for step in steps {
        for path in git.changed(&step.parent, &step.commit)? {
            let Some(service) = service_of(&path) else {
                continue;
            };
            let path = path.to_str().expect("a service file's path is UTF-8");
            let pins = changed_pins(
                git.blob(&step.parent, path)?.as_deref(),
                git.blob(&step.commit, path)?.as_deref(),
            );
            unconfirmed.note(&service, pins, step.reach == Reach::Versions);
        }
    }
    unconfirmed.save()
}

pub type Origin<'a> = &'a dyn Fn(&str, &Image, &Pin) -> Result<Option<String>>;

pub fn origin(
    registry: &Client,
    service: &str,
    image: &Image,
    pin: &Pin,
) -> Result<Option<String>> {
    let git = Git::at(GIT_DIR);
    let accepted = sync::accepted(&git)?;
    recipe::built_from(&git, &accepted, registry, service, image, pin)
}

pub fn ask_republished(
    type_name: &str,
    services: &[Service],
    registry: &Client,
    origin: Origin,
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
            if let Some(why) = origin(&key, image, &pin)? {
                println!("The probe pinned {} for {key}, and {why}.", pin.version);
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
#[path = "../../tests/services/confirm.rs"]
mod tests;
