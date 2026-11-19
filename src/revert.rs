use crate::{
    chezmoi,
    cmd::{self, As},
    fleet::{Fleet, Manager, Member, Mode, Tool},
    package::{Archive, Distro},
    tools, walk,
};
use anyhow::{Context, Result, bail};
use std::{
    ffi::OsStr,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub enum Revert {
    Machine(Machine),
    User(User),
}

pub enum Machine {
    Install(String),
    Uninstall(String),
    Etc(Etc),
    Unit(Unit),
}

pub enum Etc {
    Write(PathBuf),
    Restore { path: PathBuf, archive: Archive },
    Delete(PathBuf),
}

pub enum Unit {
    Enable(String),
    Disable(String),
}

pub enum User {
    Create,
    InitChezmoi,
    Account(Account),
    Service(Service),
    RunInstaller(String),
    Tool(ToolChange),
    ApplyFile { path: PathBuf, edited: bool },
}

pub enum Account {
    Shell(String),
    Join(String),
    Leave(String),
}

pub enum Service {
    Linger(bool),
    Unit(Unit),
}

pub enum ToolChange {
    Install { manager: Manager, tool: Tool },
    Reinstall { manager: Manager, tool: Tool },
    Uninstall { manager: Manager, name: String },
}

impl From<Machine> for Revert {
    fn from(change: Machine) -> Self {
        Revert::Machine(change)
    }
}

impl From<User> for Revert {
    fn from(change: User) -> Self {
        Revert::User(change)
    }
}

impl From<Etc> for Revert {
    fn from(change: Etc) -> Self {
        Revert::Machine(Machine::Etc(change))
    }
}

impl From<Account> for Revert {
    fn from(change: Account) -> Self {
        Revert::User(User::Account(change))
    }
}

impl From<Service> for Revert {
    fn from(change: Service) -> Self {
        Revert::User(User::Service(change))
    }
}

impl From<ToolChange> for Revert {
    fn from(change: ToolChange) -> Self {
        Revert::User(User::Tool(change))
    }
}

impl Revert {
    pub fn describe(&self) -> String {
        match self {
            Revert::Machine(Machine::Install(_)) => "install".into(),
            Revert::Machine(Machine::Uninstall(_)) => "uninstall".into(),
            Revert::Machine(Machine::Etc(Etc::Restore { .. })) => {
                "restore the package's version".into()
            }
            Revert::Machine(Machine::Etc(Etc::Delete(_))) => "delete".into(),
            Revert::Machine(Machine::Unit(unit))
            | Revert::User(User::Service(Service::Unit(unit))) => unit.describe().into(),
            Revert::User(User::Create) => "create it".into(),
            Revert::User(User::InitChezmoi) => "write its config".into(),
            Revert::User(User::Service(Service::Linger(true))) => "turn it on".into(),
            Revert::User(User::Service(Service::Linger(false))) => "turn it off".into(),
            Revert::User(User::Account(Account::Shell(shell))) => {
                format!("make {shell} the login shell")
            }
            Revert::User(User::Account(Account::Join(_))) => "join".into(),
            Revert::User(User::Account(Account::Leave(_))) => "leave".into(),
            Revert::User(User::RunInstaller(_)) => "run its installer".into(),
            Revert::User(User::Tool(ToolChange::Install { .. })) => "install".into(),
            Revert::User(User::Tool(ToolChange::Reinstall { .. })) => {
                "reinstall from the fleet's source".into()
            }
            Revert::User(User::Tool(ToolChange::Uninstall { .. })) => "uninstall".into(),
            Revert::Machine(Machine::Etc(Etc::Write(_))) | Revert::User(User::ApplyFile { .. }) => {
                "write the fleet's version".into()
            }
        }
    }

    pub fn writes_fleet_version(&self) -> bool {
        matches!(
            self,
            Revert::Machine(Machine::Etc(Etc::Write(_))) | Revert::User(User::ApplyFile { .. })
        )
    }
}

impl Etc {
    pub fn path(&self) -> &Path {
        match self {
            Etc::Write(path) | Etc::Delete(path) | Etc::Restore { path, .. } => path,
        }
    }
}

impl Unit {
    fn describe(&self) -> &'static str {
        match self {
            Unit::Enable(_) => "enable and start",
            Unit::Disable(_) => "disable and stop",
        }
    }
}

pub type Owned<'a> = (Option<usize>, &'a Revert);

pub fn run(fleet: &Fleet, reverts: &[Owned]) -> Result<()> {
    let mut machine = Vec::new();
    let mut users: Vec<Vec<&User>> = fleet.members.iter().map(|_| Vec::new()).collect();
    for (owner, revert) in reverts {
        match (revert, owner) {
            (Revert::Machine(change), _) => machine.push(change),
            (Revert::User(change), Some(i)) => users[*i].push(change),
            (Revert::User(_), None) => bail!("a user's change came without its user"),
        }
    }
    run_machine(fleet, &machine)?;
    for (member, mine) in fleet.members.iter().zip(&users) {
        if !mine.is_empty() {
            run_member(fleet, member, mine)?;
        }
    }
    Ok(())
}

fn run_machine(fleet: &Fleet, changes: &[&Machine]) -> Result<()> {
    let (mut install, mut uninstall, mut etc, mut units) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for change in changes.iter().copied() {
        match change {
            Machine::Install(name) => install.push(name.as_str()),
            Machine::Uninstall(name) => uninstall.push(name.as_str()),
            Machine::Etc(change) => etc.push(change),
            Machine::Unit(unit) => units.push(unit),
        }
    }
    let distro = Distro::here()?;
    let (sources, etc): (Vec<&Etc>, Vec<&Etc>) = etc
        .into_iter()
        .partition(|change| distro.configures(change.path()));
    for change in &sources {
        change_etc(fleet, change)?;
    }
    if !install.is_empty() {
        distro.install(&fleet.aur(), &install)?;
    }
    if !uninstall.is_empty() {
        distro.uninstall(&fleet.listed_packages(), &uninstall)?;
    }
    for change in &etc {
        change_etc(fleet, change)?;
    }
    if !sources.is_empty() || !etc.is_empty() {
        cmd::root("systemctl", &["daemon-reload"])?;
    }
    for unit in units {
        systemctl(&[], unit)?;
    }
    Ok(())
}

fn run_member(fleet: &Fleet, member: &Member, changes: &[&User]) -> Result<()> {
    let (mut account, mut linger, mut installers, mut tools) =
        (Vec::new(), None, Vec::new(), Vec::new());
    let (mut files, mut units, mut init) = (Vec::new(), Vec::new(), false);
    for change in changes.iter().copied() {
        match change {
            User::Create => return create_user(member),
            User::InitChezmoi => init = true,
            User::Account(change) => account.push(change),
            User::Service(Service::Linger(on)) => linger = Some(*on),
            User::Service(Service::Unit(unit)) => units.push(unit),
            User::RunInstaller(name) => installers.push(name),
            User::Tool(change) => tools.push(change),
            User::ApplyFile { path, .. } => files.push(path.as_os_str()),
        }
    }
    let user = member.user();
    let name = &user.name;
    if init {
        chezmoi::init(member.user(), &member.chezmoi_config(fleet))?;
    }
    for change in account {
        match change {
            Account::Shell(shell) => cmd::root("usermod", &["--shell", shell, name])?,
            Account::Join(group) => cmd::root("gpasswd", &["--add", name, "--", group])?,
            Account::Leave(group) => cmd::root("gpasswd", &["--delete", name, "--", group])?,
        }
    }
    match linger {
        Some(true) => cmd::root("loginctl", &["enable-linger", name])?,
        Some(false) => cmd::root("loginctl", &["disable-linger", name])?,
        None => {}
    }
    for name in installers {
        let installer = member
            .config
            .installers
            .iter()
            .find(|installer| &installer.name == name)
            .with_context(|| format!("{} has no installer {name}", member.path().display()))?;
        tools::run_installer(user, installer)?;
    }
    for change in tools {
        match change {
            ToolChange::Install { manager, tool } => tools::install(user, *manager, tool)?,
            ToolChange::Reinstall { manager, tool } => {
                tools::uninstall(user, *manager, tool.name())?;
                tools::install(user, *manager, tool)?;
            }
            ToolChange::Uninstall { manager, name } => tools::uninstall(user, *manager, name)?,
        }
    }
    if !files.is_empty() {
        let args = [&[OsStr::new("apply"), OsStr::new("--force")], &files[..]].concat();
        cmd::run(As::User(user), "chezmoi", &args)?;
    }
    if !units.is_empty() {
        cmd::root(
            "systemctl",
            &["start", &format!("user@{}.service", user.uid)],
        )?;
        let machine = format!("{name}@");
        let scope = ["--user", "--machine", &machine];
        cmd::root("systemctl", &[&scope[..], &["daemon-reload"]].concat())?;
        for unit in units {
            systemctl(&scope, unit)?;
        }
    }
    Ok(())
}

pub fn create_user(member: &Member) -> Result<()> {
    let mut args = vec!["--create-home"];
    if let Some(shell) = &member.config.shell {
        args.extend(["--shell", shell]);
    }
    args.extend(["--", &member.name]);
    cmd::root("useradd", &args)
}

fn systemctl(scope: &[&str], unit: &Unit) -> Result<()> {
    let (verb, unit) = match unit {
        Unit::Enable(unit) => ("enable", unit),
        Unit::Disable(unit) => ("disable", unit),
    };
    cmd::root("systemctl", &[scope, &[verb, "--now", "--", unit]].concat())
}

fn change_etc(fleet: &Fleet, change: &Etc) -> Result<()> {
    let path = change.path();
    let run = || match change {
        Etc::Write(_) => {
            let rel = path.strip_prefix(walk::ETC)?;
            let source = fleet.etc_source(rel);
            if fs::symlink_metadata(&source)?.is_symlink() {
                walk::link(path, &fs::read_link(&source)?)
            } else {
                walk::write(path, &fleet.etc_contents(rel)?, fleet.etc_mode(rel)?.0)
            }
        }
        Etc::Restore { archive, .. } => {
            let mode = Mode(fs::metadata(path)?.permissions().mode() & 0o7777);
            walk::write(path, &archive.shipped(path)?, mode.0)
        }
        Etc::Delete(_) if path.is_dir() => Ok(fs::remove_dir_all(path)?),
        Etc::Delete(_) => Ok(fs::remove_file(path)?),
    };
    run().with_context(|| format!("changing {}", path.display()))
}
