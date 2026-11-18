use crate::{cmd, ignore::Ignore, user};
use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, de};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub struct Fleet {
    pub dir: PathBuf,
    pub type_dir: PathBuf,
    pub system: System,
    pub ignore: Ignore,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct System {
    pub packages: Vec<String>,
    pub built_packages: Vec<String>,
    /// Installs the listed packages that aren't in pacman's repos. It takes pacman's `-S --needed`.
    pub aur_helper: Option<String>,
    pub shell: Option<String>,
    pub groups: Vec<String>,
    pub services: Vec<String>,
    pub user_services: Vec<String>,
    pub installers: Vec<Installer>,
    pub tools: Tools,
    /// Files in etc/ that need another mode than 644, or 755 for executables. git keeps only the
    /// executable bit.
    pub etc_modes: BTreeMap<PathBuf, Mode>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mode(pub u32);

impl Mode {
    pub fn default_for(executable: bool) -> Mode {
        Mode(if executable { 0o755 } else { 0o644 })
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

const IGNORE: &str = "ignore.txt";

impl Fleet {
    /// The machine's device type is the chezmoi profile it was set up with.
    pub fn load() -> Result<Self> {
        let data: serde_json::Value =
            serde_json::from_str(&cmd::stdout("chezmoi", &["data", "--format", "json"])?)?;
        let dir = PathBuf::from(
            data["chezmoi"]["sourceDir"]
                .as_str()
                .context("chezmoi data has no sourceDir")?,
        );
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
        let ignore = Ignore::load(&type_dir.join(IGNORE))?;
        Ok(Fleet {
            dir,
            type_dir,
            system,
            ignore,
        })
    }

    pub fn ignore_path(&self) -> PathBuf {
        self.type_dir.join(IGNORE)
    }

    pub fn package_path(&self, list: &str) -> PathBuf {
        self.dir.join("packages").join(format!("{list}.txt"))
    }

    pub fn etc_dir(&self) -> PathBuf {
        self.type_dir.join("etc")
    }

    /// `rel` is the file's path under etc/.
    pub fn etc_mode(&self, rel: &Path) -> Result<Mode> {
        if let Some(mode) = self.system.etc_modes.get(rel) {
            return Ok(*mode);
        }
        let path = self.etc_dir().join(rel);
        let meta = fs::metadata(&path).with_context(|| format!("reading {}", path.display()))?;
        Ok(Mode::default_for(meta.permissions().mode() & 0o111 != 0))
    }

    /// What the type's package lists install, leaving out the built ones.
    pub fn packages(&self) -> Result<BTreeSet<String>> {
        let mut names = BTreeSet::new();
        for list in &self.system.packages {
            names.extend(self.package_list(list)?);
        }
        Ok(names)
    }

    pub fn package_list(&self, name: &str) -> Result<Vec<String>> {
        let path = self.package_path(name);
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

pub fn expand(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => user::home().join(rest),
        None => PathBuf::from(path),
    }
}

pub fn tilde(path: &Path) -> String {
    match path.strip_prefix(user::home()) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}
