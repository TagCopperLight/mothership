use crate::{secrets::Recipient, trust::sign::Signer, util::file};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::{collections::BTreeMap, path::Path};

pub const MANIFEST: &str = "mothership.toml";

#[derive(Deserialize, Default)]
pub struct Manifest {
    version: Option<String>,
    #[serde(default)]
    sha256: BTreeMap<String, String>,
    recovery: Option<Recovery>,
    probe: Option<Probe>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Probe {
    host: String,
}

#[derive(Deserialize)]
struct Recovery {
    age: String,
    signing: String,
}

pub struct Pin<'a> {
    pub version: &'a str,
    pub sha256: &'a BTreeMap<String, String>,
}

impl Manifest {
    pub fn read(fleet_dir: &Path) -> Result<Self> {
        let path = fleet_dir.join(MANIFEST);
        let Some(text) = file::read_if_any(&path)? else {
            return Ok(Manifest::default());
        };
        toml::from_str(&text).with_context(|| format!("reading {}", path.display()))
    }

    pub fn pin(&self) -> Option<Pin<'_>> {
        self.version.as_deref().map(|version| Pin {
            version,
            sha256: &self.sha256,
        })
    }

    pub fn recovery_signer(&self) -> Result<Option<Signer>> {
        self.recovery
            .as_ref()
            .map(|recovery| Signer::recovery(&recovery.signing))
            .transpose()
    }

    pub fn probe_host(&self) -> Option<&str> {
        self.probe.as_ref().map(|probe| probe.host.as_str())
    }

    pub fn recovery(&self) -> Option<Recipient> {
        self.recovery.as_ref().map(|recovery| Recipient {
            name: crate::secrets::RECOVERY.to_owned(),
            key: recovery.age.clone(),
        })
    }
}
