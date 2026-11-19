use crate::{
    fleet::Fleet,
    scan::{self, Item},
    walk,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
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

#[derive(Default)]
pub struct Later {
    items: BTreeMap<String, Waiting>,
}

pub fn now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

impl Later {
    pub fn load() -> Result<Self> {
        Ok(Later {
            items: walk::read_json(Path::new(PATH))?,
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
        self.keep_only(fleet, current);
        walk::write_json(Path::new(PATH), &self.items)
    }

    fn keep_only(&mut self, fleet: &Fleet, current: &[&Item]) {
        let keys: BTreeSet<String> = current.iter().map(|i| scan::key(fleet, i)).collect();
        self.items.retain(|key, _| keys.contains(key));
    }
}

#[cfg(test)]
#[path = "../tests/later.rs"]
mod tests;
