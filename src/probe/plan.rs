use super::state::{Seen, State};
use crate::container::{Image, Pin, registry::Client, version::Version};
use anyhow::Result;

pub const PUBLIC_WAIT: u64 = 2 * 24 * 60 * 60;
pub const PUBLIC_EVERY: u64 = 60 * 60;

pub struct Watched {
    pub service: String,
    pub name: String,
    pub image: Image,
    pub dockerfile: Option<String>,
}

pub type Verify<'a> = &'a dyn Fn(&Watched, &Pin) -> Result<()>;

impl Watched {
    pub fn mine(&self) -> bool {
        self.image.mine()
    }

    fn key(&self) -> String {
        format!("{} {}", self.service, self.name)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Round {
    Mine,
    Everything,
}

impl Round {
    pub fn at(state: &State, now: u64) -> Round {
        match now.saturating_sub(state.public_checked) >= PUBLIC_EVERY {
            true => Round::Everything,
            false => Round::Mine,
        }
    }

    pub fn covers(self, item: &Watched) -> bool {
        self == Round::Everything || item.mine()
    }
}

#[derive(Debug, PartialEq)]
pub struct Update {
    pub service: String,
    pub name: String,
    pub from: Pin,
    pub to: Pin,
}

pub struct Found {
    pub updates: Vec<Update>,
    pub failures: Vec<String>,
}

pub fn plan(
    watched: &[Watched],
    registry: &Client,
    state: &mut State,
    now: u64,
    round: Round,
    verify: Verify,
) -> Found {
    let mut found = Found {
        updates: Vec::new(),
        failures: Vec::new(),
    };
    let keys: Vec<String> = watched.iter().map(Watched::key).collect();
    state.notices.retain(|key, _| keys.contains(key));
    if round == Round::Everything {
        state.public_checked = now;
    }
    for item in watched.iter().filter(|item| round.covers(item)) {
        if let Err(e) = watch(item, registry, state, now, verify, &mut found) {
            found
                .failures
                .push(format!("{} in {}: {e:#}", item.image.image, item.service));
        }
    }
    found
}

fn watch(
    item: &Watched,
    registry: &Client,
    state: &mut State,
    now: u64,
    verify: Verify,
    found: &mut Found,
) -> Result<()> {
    let image = &item.image;
    let current = Version::parse(&image.version)?;
    let mut newer = Vec::new();
    let mut major = None::<(u64, String)>;
    for tag in registry.tags(&image.image)? {
        let Ok(version) = Version::parse(&tag) else {
            continue;
        };
        if !version.same_shape(&current) {
            continue;
        }
        if version.major() > current.major() {
            if major.as_ref().is_none_or(|(m, _)| version.major() > *m) {
                major = Some((version.major(), tag.clone()));
            }
        } else if version.major() == current.major() && version >= current {
            newer.push((version, tag));
        }
    }
    match major {
        Some((_, tag)) => drop(state.notices.insert(
            item.key(),
            format!(
                "{} {tag} is out, and {} follows {}: move it by hand",
                image.image,
                item.service,
                current.major()
            ),
        )),
        None => drop(state.notices.remove(&item.key())),
    }
    newer.sort_by(|a, b| b.0.partial_cmp(&a.0).expect("one shape compares"));
    for (_, tag) in newer {
        let digest = registry.digest(&image.image, &tag)?;
        let key = format!("{}:{tag}", image.image);
        let seen = state
            .seen
            .entry(key)
            .and_modify(|seen| {
                if seen.digest != digest {
                    *seen = Seen {
                        digest: digest.clone(),
                        since: now,
                    };
                }
            })
            .or_insert(Seen {
                digest: digest.clone(),
                since: now,
            });
        let ripe = item.mine() || now.saturating_sub(seen.since) >= PUBLIC_WAIT;
        let to = Pin {
            version: tag,
            digest,
        };
        if to == image.pin() {
            break;
        }
        if ripe {
            verify(item, &to)?;
            found.updates.push(Update {
                service: item.service.clone(),
                name: item.name.clone(),
                from: image.pin(),
                to,
            });
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/plan.rs"]
mod tests;
