use crate::{package::aur, secrets::Storage, system::hardware::Hardware, util::file};
use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize, de};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use strum::{EnumIter, IntoStaticStr};

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct System {
    pub packages: Vec<String>,
    pub built_packages: Vec<String>,
    pub aur_helper: Option<aur::Helper>,
    pub services: Vec<String>,
    pub etc_modes: BTreeMap<PathBuf, Mode>,
    pub(super) hardware: BTreeMap<Hardware, Block>,
    pub(super) options: BTreeMap<String, Block>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(deny_unknown_fields, default)]
pub struct UserConfig {
    pub shell: Option<String>,
    pub groups: Vec<String>,
    pub user_services: Vec<String>,
    pub linger: bool,
    pub installers: Vec<Installer>,
    pub tools: Tools,
    pub ssh_from: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub(super) struct Block {
    pub(super) packages: Vec<String>,
    pub(super) built_packages: Vec<String>,
    pub(super) services: Vec<String>,
    pub(super) etc: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Machine {
    #[serde(rename = "type")]
    pub device_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
}

impl Machine {
    pub fn load() -> Result<Machine> {
        toml::from_str(&fs::read_to_string(MACHINE).with_context(|| format!("reading {MACHINE}"))?)
            .with_context(|| format!("parsing {MACHINE}"))
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let path = file::under(root, MACHINE);
        file::write(&path, toml::to_string(self)?.as_bytes(), 0o644)
            .with_context(|| format!("writing {}", path.display()))
    }
}

pub const MACHINE: &str = "/etc/mothership/machine.toml";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mode(pub u32);

impl Mode {
    pub fn default_for(executable: bool, storage: Storage) -> Mode {
        let mode = if executable { 0o755 } else { 0o644 };
        Mode(match storage {
            Storage::Clear => mode,
            Storage::Encrypted => mode & 0o700,
        })
    }
}

impl<'de> Deserialize<'de> for Mode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        u32::from_str_radix(&text, 8)
            .ok()
            .filter(|mode| *mode <= 0o7777)
            .map(Mode)
            .ok_or_else(|| de::Error::custom(format!("{text} is not a mode like \"440\"")))
    }
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Installer {
    pub name: String,
    pub run: String,
    pub creates: String,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(deny_unknown_fields, default)]
pub struct Tools {
    cargo: Vec<Tool>,
    uv: Vec<Tool>,
    bun: Vec<Tool>,
}

impl Tools {
    pub fn of(&self, manager: Manager) -> &[Tool] {
        match manager {
            Manager::Cargo => &self.cargo,
            Manager::Uv => &self.uv,
            Manager::Bun => &self.bun,
        }
    }

    pub fn find(&self, manager: Manager, name: &str) -> Option<&Tool> {
        self.of(manager).iter().find(|tool| tool.name() == name)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, IntoStaticStr, EnumIter)]
#[strum(serialize_all = "lowercase")]
pub enum Manager {
    Cargo,
    Uv,
    Bun,
}

impl Manager {
    pub fn name(self) -> &'static str {
        self.into()
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(untagged)]
pub enum Tool {
    Name(String),
    Git(GitTool),
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct GitTool {
    pub name: String,
    pub git: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

impl Tool {
    pub fn name(&self) -> &str {
        match self {
            Tool::Name(name) => name,
            Tool::Git(tool) => &tool.name,
        }
    }
}
