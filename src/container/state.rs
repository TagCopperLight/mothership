use super::Pin;
use crate::file;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

const PATH: &str = "/var/lib/mothership/services.json";

pub type Pins = BTreeMap<String, Pin>;

#[derive(Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct State {
    pub services: BTreeMap<String, Deployed>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Deployed {
    pub fleet: String,
    pub running: Pins,
    pub files: BTreeMap<PathBuf, String>,
    pub failed: BTreeSet<(String, Pin)>,
    pub problem: Option<Problem>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Problem {
    pub fleet: String,
    pub why: String,
}

impl State {
    pub fn load() -> Result<Self> {
        file::read_json(Path::new(PATH))
    }

    pub fn save(&self) -> Result<()> {
        file::write_json(Path::new(PATH), self)
    }
}

impl Deployed {
    pub fn failed_in<'a>(&self, pins: &'a Pins) -> Vec<(&'a String, &'a Pin)> {
        pins.iter()
            .filter(|(image, pin)| self.failed.contains(&((*image).clone(), (*pin).clone())))
            .collect()
    }

    pub fn problem_for(&self, fleet: &str) -> Option<&str> {
        self.problem
            .as_ref()
            .filter(|problem| problem.fleet == fleet)
            .map(|problem| problem.why.as_str())
    }
}
