use crate::util::file;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, os::unix::fs::OpenOptionsExt, path::Path};

pub const FILE: &str = "state.json";
pub const ROOT_VIEW: &str = "/var/lib/private/mothership-probe/state.json";

#[derive(Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct State {
    pub seen: BTreeMap<String, Seen>,
    pub bases: BTreeMap<String, BTreeMap<String, String>>,
    pub notices: BTreeMap<String, String>,
    pub public_checked: u64,
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

    pub fn load_as_root() -> Result<State> {
        let path = Path::new(ROOT_VIEW);
        let opened = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
            .open(path);
        let Some(opened) =
            file::if_found(opened).with_context(|| format!("opening {ROOT_VIEW}"))?
        else {
            return Ok(State::default());
        };
        if !opened.metadata()?.is_file() {
            bail!("{ROOT_VIEW} isn't a regular file");
        }
        let bytes = file::read_limited(opened, ROOT_VIEW)?;
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {ROOT_VIEW}"))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        file::write_json(path, self)
    }
}
