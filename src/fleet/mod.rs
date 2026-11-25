use crate::{
    home::chezmoi,
    package::{self},
    secrets::{self, Keys, Recipient, Storage},
    services,
    sync::ignore::Ignore,
    sync::kind::Level,
    system::hardware::{self, Hardware},
    system::user::{self, User},
    util::style,
    util::walk,
};
use anyhow::{Context, Result, bail};
use serde::de;
use std::{
    collections::BTreeSet,
    fs,
    ops::Deref,
    os::unix::fs::PermissionsExt,
    path::{Component, Path, PathBuf},
};

pub mod checkout;
mod config;
pub mod edit;
pub mod git;
pub mod homes;
mod logins;
pub mod machines;
mod manifest;
mod recipients;
pub mod stage;
pub mod sync;

pub use manifest::{MANIFEST, Manifest, Pin};

use config::Block;
pub use config::{
    GitTool, Here, Installer, MACHINE, Machine, Manager, Mode, System, Tool, Tools, UserConfig,
};
use homes::Reader;
use machines::Machines;

pub struct Repo {
    pub dir: PathBuf,
    pub type_name: String,
    pub type_dir: PathBuf,
    pub system: System,
    pub blocks: Vec<Table>,
    pub etc: BTreeSet<PathBuf>,
    pub etc_elsewhere: BTreeSet<PathBuf>,
    pub secret_etc: BTreeSet<PathBuf>,
    pub ignore: Ignore,
    pub profiles: Vec<Profile>,
    pub recovery: Option<Recipient>,
    pub services: Vec<services::Service>,
    pub probe_host: Option<String>,
    pub machines: Machines,
}

pub struct Fleet {
    pub repo: Repo,
    pub name: String,
    pub machine: Machine,
    pub members: Vec<Member>,
    admin: User,
    keys: Keys,
    root_keys: Keys,
    etc_open: bool,
}

impl Deref for Fleet {
    type Target = Repo;

    fn deref(&self) -> &Repo {
        &self.repo
    }
}

#[derive(Clone)]
pub enum Table {
    Top,
    Hardware(String),
    Option(String),
}

impl Table {
    pub fn path(&self) -> Vec<&str> {
        match self {
            Table::Top => Vec::new(),
            Table::Hardware(name) => vec!["hardware", name],
            Table::Option(name) => vec!["options", name],
        }
    }
}

#[derive(Clone)]
pub struct Profile {
    pub name: String,
    pub dir: PathBuf,
    pub config: UserConfig,
    pub logins: Option<Vec<String>>,
    pub ignore: Ignore,
}

pub struct Member {
    pub profile: Profile,
    pub chezmoi: bool,
    pub open: bool,
    user: User,
}

impl Deref for Member {
    type Target = Profile;

    fn deref(&self) -> &Profile {
        &self.profile
    }
}

pub const TYPES: &str = "types";
pub const SYSTEM_TOML: &str = "system.toml";
pub const USER_TOML: &str = "user.toml";
pub const ETC_DIR: &str = "etc";
const IGNORE: &str = "ignore.txt";
pub const USERS: &str = "users";

impl Fleet {
    pub fn load() -> Result<Self> {
        Fleet::load_at(Path::new(sync::CURRENT))
    }

    pub fn load_at(dir: &Path) -> Result<Self> {
        let here = Here::load()?;
        let machine = here.machine(&Machines::load(dir)?)?;
        let admin = checkout::admin()?;
        let repo = Repo::for_machine(dir.to_path_buf(), &machine)?;
        let mut keys = Keys::default();
        let mut root_keys = Keys::default();
        let etc_open = match secrets::root_key()? {
            Some(key) => {
                let open = repo.opens(&Reader::Root(repo.type_name.clone()), &key);
                root_keys.push(key.clone());
                keys.push(key);
                open
            }
            None => false,
        };
        let mut members = Vec::new();
        for profile in &repo.profiles {
            let Some(user) = user::by_name(&profile.name)? else {
                continue;
            };
            let open = match secrets::user_key(&user) {
                Ok(Some(key)) => {
                    let open = repo.opens(&repo.reader(profile), &key);
                    keys.push(key);
                    open
                }
                Ok(None) => false,
                Err(e) => {
                    eprintln!(
                        "{}'s secrets stay locked: {}",
                        profile.name,
                        style::clean(&format!("{e:#}"))
                    );
                    false
                }
            };
            let chezmoi = chezmoi::configured(&user, &chezmoi_config(&repo, profile, &user))?;
            members.push(Member {
                profile: profile.clone(),
                user,
                chezmoi,
                open,
            });
        }
        Ok(Fleet {
            repo,
            name: here.name,
            machine,
            admin,
            keys,
            root_keys,
            etc_open,
            members,
        })
    }

    pub fn admin(&self) -> &User {
        &self.admin
    }

    pub fn keys(&self) -> &Keys {
        &self.keys
    }

    pub fn root_keys(&self) -> &Keys {
        &self.root_keys
    }

    pub fn etc_open(&self) -> bool {
        self.etc_open
    }

    pub fn missing(&self) -> impl Iterator<Item = &Profile> {
        self.profiles
            .iter()
            .filter(|profile| !self.members.iter().any(|m| m.name == profile.name))
    }

    pub fn etc_contents(&self, rel: &Path) -> Result<Vec<u8>> {
        let source = self.etc_source(rel);
        if self.secret_etc.contains(rel) {
            return self.root_keys.decrypt(&source);
        }
        fs::read(&source).with_context(|| format!("reading {}", source.display()))
    }

    pub fn aur(&self) -> package::Aur<'_> {
        package::Aur {
            helper: self.system.aur_helper,
            admin: &self.admin,
        }
    }
}

impl Repo {
    pub fn for_machine(dir: PathBuf, machine: &Machine) -> Result<Self> {
        let hardware = hardware::detect(Path::new("/"))?;
        Repo::new(dir, &machine.device_type, &machine.options, &hardware)
    }

    pub fn new(
        dir: PathBuf,
        device_type: &str,
        options: &[String],
        hardware: &Hardware,
    ) -> Result<Self> {
        homes::check(&dir)?;
        let manifest = Manifest::read(&dir)?;
        let recovery = manifest.recovery();
        let probe_host = manifest.probe_host().map(str::to_owned);
        let type_dir = type_dir(&dir, device_type);
        let system_path = type_dir.join(SYSTEM_TOML);
        let mut system: System = read_toml(&system_path)?;

        if let Some(name) = system
            .options
            .iter()
            .find_map(|(name, b)| b.detect.as_ref().map(|_| name))
        {
            bail!(
                "[options.{name}] of {} can't detect anything: a machine's file turns it on",
                system_path.display()
            );
        }
        let mut blocks = Vec::new();
        let mut active = Vec::new();
        for (name, block) in std::mem::take(&mut system.hardware) {
            let detect = block.detect.as_ref().with_context(|| {
                format!(
                    "[hardware.{name}] of {} says nothing to detect",
                    system_path.display()
                )
            })?;
            if detect.matches(hardware) {
                blocks.push(Table::Hardware(name));
                active.push(block);
            } else {
                system.hardware.insert(name, block);
            }
        }
        for option in options {
            let block = system.options.remove(option).with_context(|| {
                format!(
                    "the machine turns on {option}, and {} has no [options.{option}]",
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
        let machines = Machines::load(&dir)?;
        machines.check(&dir)?;
        let profiles = profiles(&dir, &machines, &type_dir)?;
        let services = services::load(&type_dir)?;
        Ok(Repo {
            dir,
            type_name: device_type.to_owned(),
            type_dir,
            system,
            blocks,
            etc,
            etc_elsewhere,
            secret_etc,
            ignore,
            profiles,
            recovery,
            services,
            probe_host,
            machines,
        })
    }

    pub fn reader(&self, profile: &Profile) -> Reader {
        Reader::Home(homes::Home {
            type_name: self.type_name.clone(),
            user: profile.name.clone(),
        })
    }

    fn opens(&self, reader: &Reader, key: &age::x25519::Identity) -> bool {
        let public = secrets::public(key);
        self.machines
            .recipients(reader)
            .iter()
            .any(|r| r.key == public)
    }

    pub fn named_recovery(&self) -> Result<&Recipient> {
        self.recovery
            .as_ref()
            .with_context(|| format!("the fleet's {MANIFEST} names no recovery key"))
    }

    pub fn sealed(&self, readers: &[Reader]) -> Result<Vec<Recipient>> {
        Ok(self.machines.sealed(readers, Some(self.named_recovery()?)))
    }

    pub fn options(&self) -> Vec<String> {
        self.system.options.keys().cloned().collect()
    }

    pub fn ignore_path(&self) -> PathBuf {
        self.type_dir.join(IGNORE)
    }

    pub fn system_path(&self) -> PathBuf {
        self.type_dir.join(SYSTEM_TOML)
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

    pub fn listed_packages(&self) -> BTreeSet<String> {
        self.system
            .packages
            .iter()
            .chain(&self.system.built_packages)
            .cloned()
            .collect()
    }
}

pub fn check_name(name: &str) -> Result<()> {
    let plain = matches!(
        Path::new(name).components().collect::<Vec<_>>()[..],
        [Component::Normal(part)] if part == name
    );
    if !plain {
        bail!("\"{name}\" isn't a plain name: use one folder name, without / or ..");
    }
    Ok(())
}

pub fn users(type_dir: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for path in walk::children(&type_dir.join(USERS))? {
        if !path.is_dir() {
            bail!(
                "{} isn't a folder: {USERS}/ holds one folder per user",
                path.display()
            );
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .with_context(|| format!("{} is not a user name", path.display()))?;
        names.push(name.to_owned());
    }
    Ok(names)
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

impl Profile {
    pub fn path(&self) -> PathBuf {
        self.dir.join(USER_TOML)
    }

    pub fn ignore_path(&self) -> PathBuf {
        self.dir.join(IGNORE)
    }

    pub fn home(&self) -> PathBuf {
        self.dir.join(homes::HOME)
    }
}

fn profiles(fleet_dir: &Path, machines: &Machines, type_dir: &Path) -> Result<Vec<Profile>> {
    let mut profiles = Vec::new();
    for name in users(type_dir)? {
        let dir = type_dir.join(USERS).join(&name);
        let path = dir.join(USER_TOML);
        let config: UserConfig = read_toml(&path)?;
        profiles.push(Profile {
            logins: logins::resolve(fleet_dir, machines, &path, &config.ssh_from)?,
            config,
            ignore: Ignore::load(&dir.join(IGNORE), Level::User)?,
            name,
            dir,
        });
    }
    profiles.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(profiles)
}

pub fn check_loads(fleet_dir: &Path) -> Result<()> {
    for type_name in types(fleet_dir)? {
        Repo::new(
            fleet_dir.to_path_buf(),
            &type_name,
            &[],
            &Hardware::default(),
        )
        .with_context(|| format!("loading the {type_name} type"))?;
    }
    for (name, entry) in Machines::load(fleet_dir)?.entries() {
        Machines::check_options(fleet_dir, entry).with_context(|| format!("loading {name}"))?;
    }
    Ok(())
}

pub fn check_logins(fleet_dir: &Path) -> Result<()> {
    let machines = Machines::load(fleet_dir)?;
    for type_name in types(fleet_dir)? {
        let type_dir = type_dir(fleet_dir, &type_name);
        for user in users(&type_dir)? {
            let path = type_dir.join(USERS).join(user).join(USER_TOML);
            let config: UserConfig = read_toml(&path)?;
            let shown = path.strip_prefix(fleet_dir).unwrap_or(&path);
            logins::resolve(fleet_dir, &machines, shown, &config.ssh_from)?;
        }
    }
    Ok(())
}

pub(super) fn read_toml<T: de::DeserializeOwned>(path: &Path) -> Result<T> {
    toml::from_str(
        &fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("parsing {}", path.display()))
}

impl Member {
    pub fn user(&self) -> &User {
        &self.user
    }

    pub fn chezmoi_config(&self, fleet: &Fleet) -> chezmoi::ChezmoiConfig {
        chezmoi_config(fleet, &self.profile, &self.user)
    }

    pub fn chezmoi_root(&self, fleet: &Fleet) -> chezmoi::Root {
        let config = chezmoi::ChezmoiConfig {
            source: self.home(),
            ..self.chezmoi_config(fleet)
        };
        chezmoi::Root::new(&self.user, config)
    }
}

fn chezmoi_config(repo: &Repo, profile: &Profile, user: &User) -> chezmoi::ChezmoiConfig {
    let published = |path: PathBuf| {
        let rel = path
            .strip_prefix(&repo.dir)
            .expect("a profile lives in its fleet");
        Path::new(sync::CURRENT).join(rel)
    };
    chezmoi::ChezmoiConfig {
        source: published(profile.home()),
        identity: secrets::key_path(user),
        recipients: recipients::list_path(&repo.type_name, &profile.name),
        profile: repo.type_name.clone(),
        user: profile.name.clone(),
        logins: profile.logins.clone(),
    }
}

#[cfg(test)]
impl Fleet {
    pub fn of(repo: Repo, user: &User) -> Fleet {
        let members = repo
            .profiles
            .iter()
            .map(|profile| Member {
                profile: profile.clone(),
                user: user.clone(),
                chezmoi: false,
                open: false,
            })
            .collect();
        Fleet {
            name: "machine".to_owned(),
            machine: Machine {
                device_type: repo.type_name.clone(),
                options: Vec::new(),
            },
            repo,
            admin: user.clone(),
            keys: Keys::default(),
            root_keys: Keys::default(),
            etc_open: false,
            members,
        }
    }
}

#[cfg(test)]
#[path = "../../tests/fleet/mod.rs"]
mod tests;
