use crate::{
    cmd,
    hardware::{self, Hardware},
    ignore::{Ignore, Level, Paths},
    secrets::{self, Keys},
    user::{self, User},
    walk,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer, Serialize, de};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

pub const DIR: &str = "/var/lib/mothership/fleet";

pub struct Fleet {
    pub dir: PathBuf,
    pub type_name: String,
    pub type_dir: PathBuf,
    pub system: System,
    pub tables: Vec<Vec<String>>,
    pub etc: BTreeSet<PathBuf>,
    pub etc_elsewhere: BTreeSet<PathBuf>,
    pub secret_etc: BTreeSet<PathBuf>,
    pub ignore: Ignore,
    pub not_secret: Paths,
    pub members: Vec<Member>,
    local: Option<Local>,
}

struct Local {
    admin: User,
    keys: Keys,
    etc_open: bool,
}

pub struct Member {
    pub name: String,
    pub dir: PathBuf,
    pub config: UserConfig,
    pub ignore: Ignore,
    pub not_secret: Paths,
    pub user: Option<User>,
    pub chezmoi: bool,
    pub open: bool,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct System {
    pub packages: Vec<String>,
    pub built_packages: Vec<String>,
    pub aur_helper: Option<String>,
    pub services: Vec<String>,
    /// Files in etc/ that need another mode than 644, or 755 for executables. git keeps only the
    /// executable bit.
    pub etc_modes: BTreeMap<PathBuf, Mode>,
    hardware: BTreeMap<Hardware, Block>,
    options: BTreeMap<String, Block>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct UserConfig {
    pub shell: Option<String>,
    pub groups: Vec<String>,
    pub user_services: Vec<String>,
    pub linger: bool,
    pub installers: Vec<Installer>,
    pub tools: Tools,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct Block {
    packages: Vec<String>,
    built_packages: Vec<String>,
    services: Vec<String>,
    etc: Vec<PathBuf>,
}

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Manager {
    Cargo,
    Uv,
    Bun,
}

impl Manager {
    pub const ALL: [Manager; 3] = [Manager::Cargo, Manager::Uv, Manager::Bun];

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
const NOT_SECRET: &str = "not-secret.txt";
const USERS: &str = "users";

impl Fleet {
    pub fn load() -> Result<Self> {
        let machine: Machine = toml::from_str(
            &fs::read_to_string(MACHINE).with_context(|| format!("reading {MACHINE}"))?,
        )
        .with_context(|| format!("parsing {MACHINE}"))?;
        let dir = PathBuf::from(DIR);
        let meta = fs::metadata(&dir)
            .with_context(|| format!("reading {DIR}: clone the fleet there as its admin"))?;
        let admin = user::by_uid(meta.uid())?;
        let hardware = hardware::detect(Path::new("/"))?;
        let mut fleet = Fleet::new(dir, &machine.device_type, &machine.options, &hardware)?;
        for member in &mut fleet.members {
            member.user = user::by_name(&member.name)?;
        }
        let keys = Keys::load(
            std::iter::once(PathBuf::from(secrets::ROOT_KEY)).chain(
                fleet
                    .members
                    .iter()
                    .filter_map(|m| m.user.as_ref().map(secrets::key_path)),
            ),
        )?;
        let etc_open = secrets::opens(&fleet.recipients(), Path::new(secrets::ROOT_KEY))?;
        for member in &mut fleet.members {
            let Some(user) = &member.user else { continue };
            member.open = secrets::opens(&member.recipients(), &secrets::key_path(user))?;
            member.chezmoi = reads_fleet(member, user, &fleet.dir)?;
        }
        fleet.local = Some(Local {
            admin,
            keys,
            etc_open,
        });
        Ok(fleet)
    }

    fn local(&self) -> &Local {
        self.local.as_ref().expect("Fleet::load reads the machine")
    }

    pub fn admin(&self) -> &User {
        &self.local().admin
    }

    pub fn keys(&self) -> &Keys {
        &self.local().keys
    }

    pub fn etc_open(&self) -> bool {
        self.local().etc_open
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
        let mut system: System = read_toml(&system_path)?;

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
        let ignore = Ignore::load(&type_dir.join(IGNORE), Level::Machine)?;
        let not_secret = Paths::load(&type_dir.join(NOT_SECRET))?;
        let members = members(&type_dir.join(USERS))?;
        Ok(Fleet {
            dir,
            type_name: device_type.to_owned(),
            type_dir,
            system,
            tables,
            etc,
            etc_elsewhere,
            secret_etc,
            ignore,
            not_secret,
            members,
            local: None,
        })
    }

    pub fn ignore_path(&self) -> PathBuf {
        self.type_dir.join(IGNORE)
    }

    pub fn not_secret_path(&self) -> PathBuf {
        self.type_dir.join(NOT_SECRET)
    }

    pub fn system_path(&self) -> PathBuf {
        self.type_dir.join("system.toml")
    }

    pub fn recipients(&self) -> PathBuf {
        self.type_dir.join(secrets::RECIPIENTS)
    }

    pub fn etc_dir(&self) -> PathBuf {
        self.type_dir.join("etc")
    }

    pub fn etc_source(&self, rel: &Path) -> PathBuf {
        let path = self.etc_dir().join(rel);
        if !self.secret_etc.contains(rel) {
            return path;
        }
        let mut name = path.into_os_string();
        name.push(secrets::AGE);
        PathBuf::from(name)
    }

    pub fn etc_contents(&self, rel: &Path) -> Result<Vec<u8>> {
        let source = self.etc_source(rel);
        if self.secret_etc.contains(rel) {
            return self.keys().decrypt(&source);
        }
        fs::read(&source).with_context(|| format!("reading {}", source.display()))
    }

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

    pub fn packages(&self) -> BTreeSet<String> {
        self.system.packages.iter().cloned().collect()
    }

    /// Root writes into the checkout, and the admin commits it, so whatever root wrote goes back
    /// to the admin. git's own folder is the admin's alone.
    pub fn hand_back(&self) -> Result<()> {
        let admin = self.admin();
        let mut todo = vec![self.dir.clone()];
        while let Some(dir) = todo.pop() {
            for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
                let path = entry?.path();
                if path == self.dir.join(".git") {
                    continue;
                }
                let meta = fs::symlink_metadata(&path)?;
                if meta.uid() != admin.uid {
                    admin.own(&path)?;
                }
                if meta.is_dir() {
                    todo.push(path);
                }
            }
        }
        Ok(())
    }
}

impl Member {
    pub fn path(&self) -> PathBuf {
        self.dir.join(USER_TOML)
    }

    pub fn ignore_path(&self) -> PathBuf {
        self.dir.join(IGNORE)
    }

    pub fn not_secret_path(&self) -> PathBuf {
        self.dir.join(NOT_SECRET)
    }

    pub fn recipients(&self) -> PathBuf {
        self.dir.join(secrets::RECIPIENTS)
    }

    pub fn user(&self) -> &User {
        self.user.as_ref().expect("checked by the caller")
    }
}

const USER_TOML: &str = "user.toml";

fn members(dir: &Path) -> Result<Vec<Member>> {
    let mut members = Vec::new();
    let entries = match fs::read_dir(dir) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(members),
        entries => entries.with_context(|| format!("reading {}", dir.display()))?,
    };
    for entry in entries {
        let dir = entry?.path();
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .with_context(|| format!("{} is not a user name", dir.display()))?
            .to_owned();
        members.push(Member {
            config: read_toml(&dir.join(USER_TOML))?,
            ignore: Ignore::load(&dir.join(IGNORE), Level::User)?,
            not_secret: Paths::load(&dir.join(NOT_SECRET))?,
            name,
            dir,
            user: None,
            chezmoi: false,
            open: false,
        });
    }
    members.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(members)
}

fn read_toml<T: de::DeserializeOwned>(path: &Path) -> Result<T> {
    toml::from_str(
        &fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("parsing {}", path.display()))
}

fn reads_fleet(member: &Member, user: &User, fleet_dir: &Path) -> Result<bool> {
    let dump = cmd::stdout(
        cmd::As::User(user),
        "chezmoi",
        &["dump-config", "--format", "json"],
    )?;
    let config: serde_json::Value = serde_json::from_str(&dump)?;
    let path = |value: &serde_json::Value| value.as_str().map(PathBuf::from);
    Ok(path(&config["sourceDir"]).as_deref() == Some(fleet_dir)
        && config["encryption"] == "age"
        && path(&config["age"]["identity"]) == Some(secrets::key_path(user))
        && path(&config["age"]["recipientsFile"]) == Some(member.recipients()))
}

#[cfg(test)]
#[path = "../tests/fleet.rs"]
mod tests;
