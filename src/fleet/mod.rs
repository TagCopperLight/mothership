use crate::{
    chezmoi,
    cmd::{self, As},
    hardware::{self, Hardware},
    homes,
    ignore::{Ignore, Paths},
    kind::Level,
    package::{self},
    secrets::{self, Keys, Storage},
    style,
    user::{self, User},
    walk,
};
use anyhow::{Context, Result, bail};
use serde::de;
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

mod config;

use config::Block;
pub use config::{
    GitTool, Installer, MACHINE, Machine, Manager, Mode, System, Tool, Tools, UserConfig,
};

pub const DIR: &str = "/var/lib/mothership/fleet";

pub struct Fleet {
    pub dir: PathBuf,
    pub type_name: String,
    pub type_dir: PathBuf,
    pub system: System,
    pub blocks: Vec<Table>,
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

#[derive(Clone)]
pub enum Table {
    Top,
    Hardware(Hardware),
    Option(String),
}

impl Table {
    pub fn path(&self) -> Vec<&str> {
        match self {
            Table::Top => Vec::new(),
            Table::Hardware(piece) => vec!["hardware", piece.name()],
            Table::Option(name) => vec!["options", name],
        }
    }
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

pub const TYPES: &str = "types";
pub const SYSTEM_TOML: &str = "system.toml";
pub const USER_TOML: &str = "user.toml";
const ETC_DIR: &str = "etc";
const IGNORE: &str = "ignore.txt";
const NOT_SECRET: &str = "not-secret.txt";
pub const USERS: &str = "users";

impl Fleet {
    pub fn load() -> Result<Self> {
        let machine = Machine::load()?;
        let dir = PathBuf::from(DIR);
        let meta = fs::metadata(&dir)
            .with_context(|| format!("reading {DIR}: clone the fleet there as its admin"))?;
        let admin = user::by_uid(meta.uid())?;
        let mut fleet = Fleet::for_machine(dir, &machine)?;
        for member in &mut fleet.members {
            member.user = user::by_name(&member.name)?;
        }
        let mut keys = Keys::default();
        let etc_open = match secrets::root_key()? {
            Some(key) => {
                let open = secrets::holds(&fleet.recipients(), &secrets::public(&key))?;
                keys.push(key);
                open
            }
            None => false,
        };
        for i in 0..fleet.members.len() {
            let member = &fleet.members[i];
            let Some(user) = &member.user else { continue };
            let open = match secrets::user_key(user) {
                Ok(Some(key)) => {
                    let open = secrets::holds(&member.recipients(), &secrets::public(&key))?;
                    keys.push(key);
                    open
                }
                Ok(None) => false,
                Err(e) => {
                    eprintln!(
                        "{}'s secrets stay locked: {}",
                        member.name,
                        style::clean(&format!("{e:#}"))
                    );
                    false
                }
            };
            let chezmoi = chezmoi::configured(member.user(), &member.chezmoi_config(&fleet))?;
            (fleet.members[i].open, fleet.members[i].chezmoi) = (open, chezmoi);
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

    pub fn for_machine(dir: PathBuf, machine: &Machine) -> Result<Self> {
        let hardware = hardware::detect(Path::new("/"))?;
        Fleet::new(dir, &machine.device_type, &machine.options, &hardware)
    }

    pub fn new(
        dir: PathBuf,
        device_type: &str,
        options: &[String],
        hardware: &BTreeSet<Hardware>,
    ) -> Result<Self> {
        homes::check(&dir)?;
        let type_dir = type_dir(&dir, device_type);
        let system_path = type_dir.join(SYSTEM_TOML);
        let mut system: System = read_toml(&system_path)?;

        let mut blocks = Vec::new();
        let mut active = Vec::new();
        for piece in hardware {
            let block = system.hardware.remove(piece).with_context(|| {
                format!(
                    "this machine has {0}, and {1} has no [hardware.{0}]",
                    piece.name(),
                    system_path.display()
                )
            })?;
            blocks.push(Table::Hardware(*piece));
            active.push(block);
        }
        for option in options {
            let block = system.options.remove(option).with_context(|| {
                format!(
                    "{MACHINE} turns on {option}, and {} has no [options.{option}]",
                    system_path.display()
                )
            })?;
            blocks.push(Table::Option(option.clone()));
            active.push(block);
        }
        let inactive: Vec<&Block> = system
            .hardware
            .values()
            .chain(system.options.values())
            .collect();

        let (mut all, mut secret_etc) = (BTreeSet::new(), BTreeSet::new());
        for file in etc_files(&type_dir)? {
            if file.storage == Storage::Encrypted {
                secret_etc.insert(file.rel.clone());
            }
            if !all.insert(file.rel.clone()) {
                bail!("etc/{} is both in clear and encrypted", file.rel.display());
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
            blocks,
            etc,
            etc_elsewhere,
            secret_etc,
            ignore,
            not_secret,
            members,
            local: None,
        })
    }

    pub fn options(&self) -> Vec<String> {
        self.system.options.keys().cloned().collect()
    }

    pub fn ignore_path(&self) -> PathBuf {
        self.type_dir.join(IGNORE)
    }

    pub fn not_secret_path(&self) -> PathBuf {
        self.type_dir.join(NOT_SECRET)
    }

    pub fn system_path(&self) -> PathBuf {
        self.type_dir.join(SYSTEM_TOML)
    }

    pub fn recipients(&self) -> PathBuf {
        self.type_dir.join(secrets::RECIPIENTS)
    }

    pub fn etc_dir(&self) -> PathBuf {
        self.type_dir.join(ETC_DIR)
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
        let storage = match self.secret_etc.contains(rel) {
            true => Storage::Encrypted,
            false => Storage::Clear,
        };
        Ok(Mode::default_for(
            meta.permissions().mode() & 0o111 != 0,
            storage,
        ))
    }

    pub fn packages(&self) -> BTreeSet<String> {
        self.system.packages.iter().cloned().collect()
    }

    pub fn aur(&self) -> package::Aur<'_> {
        package::Aur {
            helper: self.system.aur_helper,
            admin: self.admin(),
        }
    }

    pub fn listed_packages(&self) -> BTreeSet<String> {
        self.system
            .packages
            .iter()
            .chain(&self.system.built_packages)
            .cloned()
            .collect()
    }

    pub fn unsaved(&self) -> Result<Vec<String>> {
        let dir = self.dir.to_string_lossy();
        let args = ["-C", &dir, "status", "--porcelain=v2", "--branch"];
        let status = cmd::stdout(As::User(self.admin()), "git", &args)?;
        let changed = status.lines().filter(|l| !l.starts_with('#')).count();
        let ahead: usize = status
            .lines()
            .find_map(|l| l.strip_prefix("# branch.ab +"))
            .and_then(|ab| ab.split_whitespace().next())
            .map(str::parse)
            .transpose()?
            .unwrap_or(0);
        let mut unsaved = Vec::new();
        if !status.lines().any(|l| l.starts_with("# branch.upstream ")) {
            unsaved.push("no upstream branch to push to".to_owned());
        }
        if changed > 0 {
            unsaved.push(format!("{} not committed", style::count(changed, "change")));
        }
        if ahead > 0 {
            unsaved.push(format!("{} not pushed", style::count(ahead, "commit")));
        }
        Ok(unsaved)
    }

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

pub fn type_dir(fleet_dir: &Path, name: &str) -> PathBuf {
    fleet_dir.join(TYPES).join(name)
}

pub struct EtcFile {
    pub rel: PathBuf,
    pub source: PathBuf,
    pub storage: Storage,
}

pub fn etc_files(type_dir: &Path) -> Result<Vec<EtcFile>> {
    let dir = type_dir.join(ETC_DIR);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    Ok(walk::files(&dir)?
        .into_iter()
        .map(|file| {
            let source = dir.join(&file);
            match file.to_str().and_then(|f| f.strip_suffix(secrets::AGE)) {
                Some(rel) => EtcFile {
                    rel: PathBuf::from(rel),
                    source,
                    storage: Storage::Encrypted,
                },
                None => EtcFile {
                    rel: file,
                    source,
                    storage: Storage::Clear,
                },
            }
        })
        .collect())
}

pub fn types(dir: &Path) -> Result<Vec<String>> {
    let types = dir.join(TYPES);
    let mut names = Vec::new();
    for entry in fs::read_dir(&types).with_context(|| format!("reading {}", types.display()))? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    Ok(names)
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

fn members(dir: &Path) -> Result<Vec<Member>> {
    let mut members = Vec::new();
    for dir in walk::children(dir)? {
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

impl Member {
    pub fn chezmoi_config(&self, fleet: &Fleet) -> chezmoi::ChezmoiConfig {
        chezmoi::ChezmoiConfig {
            source: homes::source(&fleet.dir, &self.name),
            identity: secrets::key_path(self.user()),
            recipients: self.recipients(),
            profile: fleet.type_name.clone(),
            user: self.name.clone(),
            fleet: fleet.dir.clone(),
        }
    }

    pub fn chezmoi_root(&self, fleet: &Fleet) -> chezmoi::Root {
        chezmoi::Root::new(self.user(), self.chezmoi_config(fleet))
    }
}

#[cfg(test)]
#[path = "../../tests/fleet.rs"]
mod tests;
