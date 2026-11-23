use crate::{
    fleet::{Fleet, Manager, Member, Mode, Tool},
    home::chezmoi,
    home::tools,
    package::{Archive, Distro},
    services,
    util::cmd::{self, As},
    util::file,
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
    CreateUser { name: String, shell: Option<String> },
    Container(services::Action),
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
    Shell(String),
    Join(String),
    Leave(String),
    Linger(bool),
    Unit(Unit),
    InstallTool { manager: Manager, tool: Tool },
    ReinstallTool { manager: Manager, tool: Tool },
    UninstallTool { manager: Manager, name: String },
    RunInstaller(String),
    InitChezmoi,
    ApplyFile { path: PathBuf, edited: bool },
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

impl From<services::Action> for Revert {
    fn from(action: services::Action) -> Self {
        Revert::Machine(Machine::Container(action))
    }
}

impl From<Etc> for Revert {
    fn from(change: Etc) -> Self {
        Revert::Machine(Machine::Etc(change))
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
            Revert::Machine(Machine::Unit(unit)) | Revert::User(User::Unit(unit)) => {
                unit.describe().into()
            }
            Revert::Machine(Machine::CreateUser { .. }) => "create it".into(),
            Revert::Machine(Machine::Container(action)) => action.describe().into(),
            Revert::User(User::InitChezmoi) => "write its config".into(),
            Revert::User(User::Linger(true)) => "turn it on".into(),
            Revert::User(User::Linger(false)) => "turn it off".into(),
            Revert::User(User::Shell(shell)) => format!("make {shell} the login shell"),
            Revert::User(User::Join(_)) => "join".into(),
            Revert::User(User::Leave(_)) => "leave".into(),
            Revert::User(User::RunInstaller(_)) => "run its installer".into(),
            Revert::User(User::InstallTool { .. }) => "install".into(),
            Revert::User(User::ReinstallTool { .. }) => "reinstall from the fleet's source".into(),
            Revert::User(User::UninstallTool { .. }) => "uninstall".into(),
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

impl User {
    pub fn needs_service_manager(&self) -> bool {
        matches!(self, User::Linger(_) | User::Unit(_))
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
    let (mut install, mut uninstall, mut etc, mut units, mut users, mut containers) = (
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    for change in changes.iter().copied() {
        match change {
            Machine::Install(name) => install.push(name.as_str()),
            Machine::Uninstall(name) => uninstall.push(name.as_str()),
            Machine::Etc(change) => etc.push(change),
            Machine::Unit(unit) => units.push(unit),
            Machine::CreateUser { name, shell } => users.push((name, shell)),
            Machine::Container(action) => containers.push(action),
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
    let (removals, containers): (Vec<&services::Action>, Vec<&services::Action>) = containers
        .into_iter()
        .partition(|action| action.takes_away());
    if !removals.is_empty() {
        services::run(fleet, &removals)?;
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
    for (name, shell) in users {
        create_user(name, shell.as_deref())?;
    }
    if !containers.is_empty() {
        services::run(fleet, &containers)?;
    }
    Ok(())
}

fn run_member(fleet: &Fleet, member: &Member, changes: &[&User]) -> Result<()> {
    let user = member.user();
    let name = &user.name;
    let (mut account, mut linger, mut installers, mut tools) =
        (Vec::new(), None, Vec::new(), Vec::new());
    let (mut files, mut units, mut init) = (Vec::new(), Vec::new(), false);
    for change in changes.iter().copied() {
        match change {
            User::InitChezmoi => init = true,
            User::Shell(shell) => account.push(("usermod", vec!["--shell", shell, name])),
            User::Join(group) => account.push(("gpasswd", vec!["--add", name, "--", group])),
            User::Leave(group) => account.push(("gpasswd", vec!["--delete", name, "--", group])),
            User::Linger(on) => linger = Some(*on),
            User::Unit(unit) => units.push(unit),
            User::RunInstaller(name) => installers.push(name),
            User::InstallTool { .. } | User::ReinstallTool { .. } | User::UninstallTool { .. } => {
                tools.push(change)
            }
            User::ApplyFile { path, .. } => files.push(path.as_os_str()),
        }
    }
    if init {
        chezmoi::init(member.user(), &member.chezmoi_config(fleet))?;
    }
    for (program, args) in account {
        cmd::root(program, &args)?;
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
            User::InstallTool { manager, tool } => tools::install(user, *manager, tool)?,
            User::ReinstallTool { manager, tool } => {
                tools::uninstall(user, *manager, tool.name())?;
                tools::install(user, *manager, tool)?;
            }
            User::UninstallTool { manager, name } => tools::uninstall(user, *manager, name)?,
            _ => unreachable!("only tool changes are collected"),
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

pub fn create_user(name: &str, shell: Option<&str>) -> Result<()> {
    let mut args = vec!["--create-home"];
    if let Some(shell) = shell {
        args.extend(["--shell", shell]);
    }
    args.extend(["--", name]);
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
            let rel = path.strip_prefix(file::ETC)?;
            let source = fleet.etc_source(rel);
            if fs::symlink_metadata(&source)?.is_symlink() {
                file::link(path, &fs::read_link(&source)?)
            } else {
                file::write(path, &fleet.etc_contents(rel)?, fleet.etc_mode(rel)?.0)
            }
        }
        Etc::Restore { archive, .. } => {
            let mode = Mode(fs::metadata(path)?.permissions().mode() & 0o7777);
            file::write(path, &archive.shipped(path)?, mode.0)
        }
        Etc::Delete(_) if path.is_dir() => Ok(fs::remove_dir_all(path)?),
        Etc::Delete(_) => Ok(fs::remove_file(path)?),
    };
    run().with_context(|| format!("changing {}", path.display()))
}
