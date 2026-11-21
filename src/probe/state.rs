use crate::file;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

pub const FILE: &str = "state.json";
pub const ROOT_VIEW: &str = "/var/lib/private/mothership-probe/state.json";

#[derive(Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct State {
    pub seen: BTreeMap<String, Seen>,
    pub bases: BTreeMap<String, BTreeMap<String, String>>,
    pub notices: Vec<String>,
    pub problem: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Seen {
    pub digest: String,
    pub since: u64,
}

impl State {
    pub fn load(path: &Path) -> Result<State> {
        file::read_json(path)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        file::write_json(path, self)
    }
}
