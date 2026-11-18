use crate::{
    cmd,
    hardware::{self, Hardware},
    ignore::Ignore,
    secrets::{self, Secrets},
    user, walk,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer, Serialize, de};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub struct Fleet {
    pub dir: PathBuf,
    pub type_dir: PathBuf,
    /// With the blocks of this machine's hardware and options merged in.
    pub system: System,
    /// Where `system` comes from in system.toml: the top level, then each block merged in.
    pub tables: Vec<Vec<String>>,
    /// The files under etc/ this machine gets, by their path under /etc.
    pub etc: BTreeSet<PathBuf>,
    /// The files under etc/ that belong to blocks this machine doesn't have.
    pub etc_elsewhere: BTreeSet<PathBuf>,
    /// The files under etc/ the fleet keeps encrypted, whichever machines get them.
    pub secret_etc: BTreeSet<PathBuf>,
    pub ignore: Ignore,
    /// Read from chezmoi's config by `load`.
    secrets: Option<Secrets>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct System {
    pub packages: Vec<String>,
    /// Packages other repos build: checked, never installed.
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
    hardware: BTreeMap<Hardware, Block>,
    options: BTreeMap<String, Block>,
}

/// What a piece of hardware or an option adds to the type.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct Block {
    packages: Vec<String>,
    built_packages: Vec<String>,
    services: Vec<String>,
    /// Files under etc/ that only machines with the block get.
    etc: Vec<PathBuf>,
}

/// What the install asks, kept on the machine.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Machine {
    #[serde(rename = "type")]
    device_type: String,
    #[serde(default)]
    options: Vec<String>,
}

pub const MACHINE: &str = "/etc/mothership/machine.toml";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mode(pub u32);

impl Mode {
    /// A secret is for its owner only.
    pub fn default_for(executable: bool, secret: bool) -> Mode {
        let mode = if executable { 0o755 } else { 0o644 };
        Mode(if secret { mode & 0o700 } else { mode })
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
#[serde(deny_unknown_fields)]
pub struct Installer {
    pub name: String,
    /// A shell command, run as the user.
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

/// What installs tools outside pacman, as the user.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Manager {
    Cargo,
    Uv,
    Bun,
}

impl Manager {
    pub const ALL: [Manager; 3] = [Manager::Cargo, Manager::Uv, Manager::Bun];

    /// The key under `[tools]`.
    pub fn name(self) -> &'static str {
        match self {
            Manager::Cargo => "cargo",
            Manager::Uv => "uv",
            Manager::Bun => "bun",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(untagged)]
pub enum Tool {
    /// From the manager's registry.
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

const IGNORE: &str = "ignore.txt";

impl Fleet {
    pub fn load() -> Result<Self> {
        let machine: Machine = toml::from_str(
            &fs::read_to_string(MACHINE).with_context(|| format!("reading {MACHINE}"))?,
        )
        .with_context(|| format!("parsing {MACHINE}"))?;
        let config: serde_json::Value = serde_json::from_str(&cmd::stdout(
            "chezmoi",
            &["dump-config", "--format", "json"],
        )?)?;
        let dir = PathBuf::from(
            config["sourceDir"]
                .as_str()
                .context("chezmoi's config has no sourceDir")?,
        );
        let hardware = hardware::detect(Path::new("/"))?;
        let mut fleet = Fleet::new(dir, &machine.device_type, &machine.options, &hardware)?;
        fleet.secrets = Some(Secrets::new(&config, &fleet.type_dir)?);
        Ok(fleet)
    }

    pub fn secrets(&self) -> &Secrets {
        self.secrets.as_ref().expect("Fleet::load reads them")
    }

    /// Every piece of hardware the machine has needs a block, even an empty one, so that new
    /// hardware never goes without what it needs unnoticed.
    pub fn new(
        dir: PathBuf,
        device_type: &str,
        options: &[String],
        hardware: &BTreeSet<Hardware>,
    ) -> Result<Self> {
        let type_dir = dir.join("types").join(device_type);
        let system_path = type_dir.join("system.toml");
        let mut system: System = toml::from_str(
            &fs::read_to_string(&system_path)
                .with_context(|| format!("reading {}", system_path.display()))?,
        )
        .with_context(|| format!("parsing {}", system_path.display()))?;

        let mut tables = vec![vec![]];
        let mut active = Vec::new();
        for piece in hardware {
            let block = system.hardware.remove(piece).with_context(|| {
                format!(
                    "this machine has {0}, and {1} has no [hardware.{0}]",
                    piece.name(),
                    system_path.display()
                )
            })?;
            tables.push(vec!["hardware".into(), piece.name().into()]);
            active.push(block);
        }
        for option in options {
            let block = system.options.remove(option).with_context(|| {
                format!(
                    "{MACHINE} turns on {option}, and {} has no [options.{option}]",
                    system_path.display()
                )
            })?;
            tables.push(vec!["options".into(), option.clone()]);
            active.push(block);
        }
        let inactive: Vec<&Block> = system
            .hardware
            .values()
            .chain(system.options.values())
            .collect();

        let (mut all, mut secret_etc) = (BTreeSet::new(), BTreeSet::new());
        for file in walk::files(&type_dir.join("etc"))? {
            let rel = match file.to_str().and_then(|f| f.strip_suffix(secrets::AGE)) {
                Some(rel) => {
                    secret_etc.insert(PathBuf::from(rel));
                    PathBuf::from(rel)
                }
                None => file,
            };
            if !all.insert(rel.clone()) {
                bail!("etc/{} is both in clear and encrypted", rel.display());
            }
        }
        let mut owned = BTreeSet::new();
        for file in active
            .iter()
            .chain(inactive.iter().copied())
            .flat_map(|b| &b.etc)
        {
            if !all.contains(file) {
                bail!(
                    "{} names etc/{}, which isn't there",
                    system_path.display(),
                    file.display()
                );
            }
            if !owned.insert(file.clone()) {
                bail!(
                    "two blocks of {} own etc/{}",
                    system_path.display(),
                    file.display()
                );
            }
        }
        let etc_elsewhere: BTreeSet<PathBuf> = inactive
            .iter()
            .flat_map(|b| b.etc.iter().cloned())
            .collect();
        let etc = all.difference(&etc_elsewhere).cloned().collect();

        for block in active {
            system.packages.extend(block.packages);
            system.built_packages.extend(block.built_packages);
            system.services.extend(block.services);
        }
        let ignore = Ignore::load(&type_dir.join(IGNORE))?;
        Ok(Fleet {
            dir,
            type_dir,
            system,
            tables,
            etc,
            etc_elsewhere,
            secret_etc,
            ignore,
            secrets: None,
        })
    }

    pub fn ignore_path(&self) -> PathBuf {
        self.type_dir.join(IGNORE)
    }

    pub fn etc_dir(&self) -> PathBuf {
        self.type_dir.join("etc")
    }

    /// The fleet's file for /etc/`rel`.
    pub fn etc_source(&self, rel: &Path) -> PathBuf {
        let path = self.etc_dir().join(rel);
        if !self.secret_etc.contains(rel) {
            return path;
        }
        let mut name = path.into_os_string();
        name.push(secrets::AGE);
        PathBuf::from(name)
    }

    /// What apply writes to /etc/`rel`, decrypted.
    pub fn etc_contents(&self, rel: &Path) -> Result<Vec<u8>> {
        let source = self.etc_source(rel);
        if self.secret_etc.contains(rel) {
            return self.secrets().decrypt(&source);
        }
        fs::read(&source).with_context(|| format!("reading {}", source.display()))
    }

    /// `rel` is the file's path under /etc.
    pub fn etc_mode(&self, rel: &Path) -> Result<Mode> {
        if let Some(mode) = self.system.etc_modes.get(rel) {
            return Ok(*mode);
        }
        let path = self.etc_source(rel);
        let meta = fs::metadata(&path).with_context(|| format!("reading {}", path.display()))?;
        Ok(Mode::default_for(
            meta.permissions().mode() & 0o111 != 0,
            self.secret_etc.contains(rel),
        ))
    }

    /// What apply installs, leaving out the built packages.
    pub fn packages(&self) -> BTreeSet<String> {
        self.system.packages.iter().cloned().collect()
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

#[cfg(test)]
#[path = "../tests/fleet.rs"]
mod tests;
