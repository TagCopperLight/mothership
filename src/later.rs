use crate::{
    fleet::Fleet,
    scan::{self, Item},
    walk,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const DAY: u64 = 24 * 60 * 60;
pub const LATER: u64 = 7 * DAY;
const PATH: &str = "/var/lib/mothership/later.json";

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct Waiting {
    pub first: u64,
    last: u64,
}

pub struct Later {
    path: PathBuf,
    items: BTreeMap<String, Waiting>,
}

pub fn now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

impl Later {
    pub fn load() -> Result<Self> {
        Later::at(Path::new(PATH))
    }

    pub fn at(path: &Path) -> Result<Self> {
        let items = match fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("parsing {}", path.display()))?,
            Err(e) if e.kind() == ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Ok(Later {
            path: path.to_path_buf(),
            items,
        })
    }

    pub fn get(&self, fleet: &Fleet, item: &Item) -> Option<Waiting> {
        self.items.get(&scan::key(fleet, item)).copied()
    }

    pub fn due(&self, fleet: &Fleet, item: &Item, now: u64) -> bool {
        self.days_left(fleet, item, now).is_none()
    }

    pub fn days_left(&self, fleet: &Fleet, item: &Item, now: u64) -> Option<u64> {
        let waiting = self.get(fleet, item)?;
        let back = waiting.last + LATER;
        (now < back).then(|| (back - now).div_ceil(DAY))
    }

    pub fn put_off(&mut self, fleet: &Fleet, item: &Item, now: u64) {
        let waiting = self.items.entry(scan::key(fleet, item)).or_insert(Waiting {
            first: now,
            last: now,
        });
        waiting.last = now;
    }

    pub fn remove(&mut self, fleet: &Fleet, item: &Item) {
        self.items.remove(&scan::key(fleet, item));
    }

    pub fn save(mut self, fleet: &Fleet, current: &[&Item]) -> Result<()> {
        let keys: BTreeSet<String> = current.iter().map(|i| scan::key(fleet, i)).collect();
        self.items.retain(|key, _| keys.contains(key));
        fs::create_dir_all(self.path.parent().expect("has a parent"))?;
        walk::write(
            &self.path,
            serde_json::to_string_pretty(&self.items)?.as_bytes(),
            0o644,
        )
        .with_context(|| format!("writing {}", self.path.display()))
    }
}

#[cfg(test)]
#[path = "../tests/later.rs"]
mod tests;
