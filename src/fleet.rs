use crate::{cmd, ignore::Ignore};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

/// The fleet repo, seen from the device type this machine runs.
pub struct Fleet {
    pub dir: PathBuf,
    pub type_dir: PathBuf,
    pub system: System,
    pub ignore: Ignore,
}

/// `types/<type>/system.toml`: what the device type has outside chezmoi's files.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct System {
    pub packages: Vec<String>,
    pub built_packages: Vec<String>,
    pub shell: Option<String>,
    pub groups: Vec<String>,
    pub services: Vec<String>,
    pub user_services: Vec<String>,
    pub installers: Vec<Installer>,
    pub tools: Tools,
}

#[derive(Deserialize)]
pub struct Installer {
    pub name: String,
    pub creates: String,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct Tools {
    pub cargo: Vec<Tool>,
    pub uv: Vec<Tool>,
    pub bun: Vec<Tool>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum Tool {
    Name(String),
    Source { name: String },
}

impl Tool {
    pub fn name(&self) -> &str {
        match self {
            Tool::Name(name) | Tool::Source { name } => name,
        }
    }
}

impl Fleet {
    /// The machine's device type is the chezmoi profile it was set up with.
    pub fn load() -> Result<Self> {
        let dir = PathBuf::from(cmd::stdout("chezmoi", &["source-path"])?.trim());
        let data: serde_json::Value =
            serde_json::from_str(&cmd::stdout("chezmoi", &["data", "--format", "json"])?)?;
        let device_type = data["profile"]
            .as_str()
            .context("chezmoi data has no profile")?;
        let type_dir = dir.join("types").join(device_type);
        let system_path = type_dir.join("system.toml");
        let system = toml::from_str(
            &fs::read_to_string(&system_path)
                .with_context(|| format!("reading {}", system_path.display()))?,
        )
        .with_context(|| format!("parsing {}", system_path.display()))?;
        let ignore = Ignore::load(&type_dir.join("ignore.txt"))?;
        Ok(Fleet {
            dir,
            type_dir,
            system,
            ignore,
        })
    }

    /// Package names in `packages/<name>.txt`.
    pub fn package_list(&self, name: &str) -> Result<Vec<String>> {
        let path = self.dir.join("packages").join(format!("{name}.txt"));
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        Ok(text
            .lines()
            .filter_map(|line| line.split('#').next())
            .flat_map(str::split_whitespace)
            .map(str::to_owned)
            .collect())
    }
}

pub fn home() -> PathBuf {
    PathBuf::from(env::var_os("HOME").expect("HOME is set"))
}

pub fn expand(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None => PathBuf::from(path),
    }
}

/// How a path is written in reports and in ignore.txt.
pub fn tilde(path: &Path) -> String {
    match path.strip_prefix(home()) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}
