use crate::{
    cmd::{self, As},
    fleet::{self, Fleet, Manager, Member, Mode, Tool},
    package::{Archive, Distro},
    tools, user,
};
use anyhow::{Context, Result};
use std::{
    fs,
    io::Write,
    os::unix::{self, fs::PermissionsExt},
    path::{Path, PathBuf},
};

pub enum Revert {
    Install(String),
    Uninstall(String),
    InstallTool { manager: Manager, tool: Tool },
    ReinstallTool { manager: Manager, tool: Tool },
    UninstallTool { manager: Manager, name: String },
    RunInstaller(String),
    WriteEtc(PathBuf),
    RestoreEtc { path: PathBuf, archive: Archive },
    DeleteEtc(PathBuf),
    CreateUser,
    InitChezmoi,
    Linger(bool),
    Shell(String),
    Join(String),
    Leave(String),
    Enable { unit: String, user: bool },
    Disable { unit: String, user: bool },
    ApplyFile { path: PathBuf, edited: bool },
}

impl Revert {
    pub fn describe(&self) -> String {
        match self {
            Revert::Install(_) | Revert::InstallTool { .. } => "install".into(),
            Revert::Uninstall(_) | Revert::UninstallTool { .. } => "uninstall".into(),
            Revert::ReinstallTool { .. } => "reinstall from the fleet's source".into(),
            Revert::RunInstaller(_) => "run its installer".into(),
            Revert::WriteEtc(_) | Revert::ApplyFile { .. } => "write the fleet's version".into(),
            Revert::RestoreEtc { .. } => "restore the package's version".into(),
            Revert::DeleteEtc(_) => "delete".into(),
            Revert::CreateUser => "create it".into(),
            Revert::InitChezmoi => "set it up from the fleet's config template".into(),
            Revert::Linger(true) => "turn it on".into(),
            Revert::Linger(false) => "turn it off".into(),
            Revert::Shell(shell) => format!("make {shell} the login shell"),
            Revert::Join(_) => "join".into(),
            Revert::Leave(_) => "leave".into(),
            Revert::Enable { .. } => "enable and start".into(),
            Revert::Disable { .. } => "disable and stop".into(),
        }
    }
}

pub type Owned<'a> = (Option<usize>, &'a Revert);

/// The machine goes first, then each user. Packages go first, after the sources they come
/// from, since the rest may come from them, and installers before tools, since bun comes from
/// one. User units go last, after chezmoi writes theirs.
pub fn run(fleet: &Fleet, reverts: &[Owned]) -> Result<()> {
    let machine: Vec<&Revert> = reverts
        .iter()
        .filter(|(member, _)| member.is_none())
        .map(|(_, revert)| *revert)
        .collect();
    run_machine(fleet, &machine)?;
    for (i, member) in fleet.members.iter().enumerate() {
        let mine: Vec<&Revert> = reverts
            .iter()
            .filter(|(owner, _)| *owner == Some(i))
            .map(|(_, revert)| *revert)
            .collect();
        if !mine.is_empty() {
            run_member(member, &mine)?;
        }
    }
    Ok(())
}

fn run_machine(fleet: &Fleet, reverts: &[&Revert]) -> Result<()> {
    let (mut install, mut uninstall, mut etc, mut system) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for revert in reverts.iter().copied() {
        match revert {
            Revert::Install(name) => install.push(name.as_str()),
            Revert::Uninstall(name) => uninstall.push(name.as_str()),
            Revert::WriteEtc(_) | Revert::RestoreEtc { .. } | Revert::DeleteEtc(_) => {
                etc.push(revert)
            }
            Revert::Enable { .. } | Revert::Disable { .. } => system.push(revert),
            _ => unreachable!("a user's revert"),
        }
    }
    let distro = Distro::here()?;
    let (sources, etc): (Vec<&Revert>, Vec<&Revert>) = etc
        .into_iter()
        .partition(|revert| distro.configures(etc_path(revert)));
    for revert in &sources {
        change_etc(fleet, revert)?;
    }
    if !install.is_empty() {
        distro.install(fleet, &install)?;
    }
    if !uninstall.is_empty() {
        distro.uninstall(&uninstall)?;
    }
    for revert in &etc {
        change_etc(fleet, revert)?;
    }
    if !sources.is_empty() || !etc.is_empty() {
        cmd::root("systemctl", &["daemon-reload"])?;
    }
    for revert in system {
        systemctl(&[], revert)?;
    }
    Ok(())
}

fn run_member(member: &Member, reverts: &[&Revert]) -> Result<()> {
    if reverts.iter().any(|r| matches!(r, Revert::CreateUser)) {
        return user::create(member);
    }
    let user = member.user();
    let name = &user.name;
    let (mut account, mut installers, mut tools) = (Vec::new(), Vec::new(), Vec::new());
    let (mut files, mut units, mut init) = (Vec::new(), Vec::new(), false);
    for revert in reverts.iter().copied() {
        match revert {
            Revert::InitChezmoi => init = true,
            Revert::Shell(_) | Revert::Join(_) | Revert::Leave(_) | Revert::Linger(_) => {
                account.push(revert)
            }
            Revert::RunInstaller(name) => installers.push(name),
            Revert::InstallTool { .. }
            | Revert::ReinstallTool { .. }
            | Revert::UninstallTool { .. } => tools.push(revert),
            Revert::ApplyFile { path, .. } => files.push(path.to_string_lossy()),
            Revert::Enable { .. } | Revert::Disable { .. } => units.push(revert),
            _ => unreachable!("the machine's revert"),
        }
    }
    if init {
        cmd::run(As::User(user), "chezmoi", &["init", "--source", fleet::DIR])?;
    }
    for revert in account {
        match revert {
            Revert::Shell(shell) => cmd::root("usermod", &["--shell", shell, name])?,
            Revert::Join(group) => cmd::root("gpasswd", &["--add", name, group])?,
            Revert::Leave(group) => cmd::root("gpasswd", &["--delete", name, group])?,
            Revert::Linger(true) => cmd::root("loginctl", &["enable-linger", name])?,
            Revert::Linger(false) => cmd::root("loginctl", &["disable-linger", name])?,
            _ => unreachable!("sorted above"),
        }
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
    for revert in tools {
        match revert {
            Revert::InstallTool { manager, tool } => tools::install(user, *manager, tool)?,
            Revert::ReinstallTool { manager, tool } => {
                tools::uninstall(user, *manager, tool.name())?;
                tools::install(user, *manager, tool)?;
            }
            Revert::UninstallTool { manager, name } => tools::uninstall(user, *manager, name)?,
            _ => unreachable!("sorted above"),
        }
    }
    if !files.is_empty() {
        let files: Vec<&str> = files.iter().map(|f| f.as_ref()).collect();
        cmd::run(
            As::User(user),
            "chezmoi",
            &[&["apply", "--force"], &files[..]].concat(),
        )?;
    }
    if !units.is_empty() {
        // --machine talks to the user's manager, which only runs once someone logs in or linger
        // is on. At a machine's first start, nobody has logged in yet.
        cmd::root(
            "systemctl",
            &["start", &format!("user@{}.service", user.uid)],
        )?;
        let machine = format!("{name}@");
        let scope = ["--user", "--machine", &machine];
        cmd::root("systemctl", &[&scope[..], &["daemon-reload"]].concat())?;
        for revert in units {
            systemctl(&scope, revert)?;
        }
    }
    Ok(())
}

fn systemctl(scope: &[&str], revert: &Revert) -> Result<()> {
    let (verb, unit) = match revert {
        Revert::Enable { unit, .. } => ("enable", unit),
        Revert::Disable { unit, .. } => ("disable", unit),
        _ => unreachable!("sorted above"),
    };
    cmd::root("systemctl", &[scope, &[verb, "--now", unit]].concat())
}

fn etc_path(revert: &Revert) -> &Path {
    let (Revert::WriteEtc(path) | Revert::DeleteEtc(path) | Revert::RestoreEtc { path, .. }) =
        revert
    else {
        unreachable!("sorted above")
    };
    path
}

fn change_etc(fleet: &Fleet, revert: &Revert) -> Result<()> {
    let path = etc_path(revert);
    let change = || match revert {
        Revert::WriteEtc(_) => {
            let rel = path.strip_prefix("/etc")?;
            let source = fleet.etc_source(rel);
            if fs::symlink_metadata(&source)?.is_symlink() {
                link(path, &fs::read_link(&source)?)
            } else {
                write(path, &fleet.etc_contents(rel)?, fleet.etc_mode(rel)?)
            }
        }
        Revert::RestoreEtc { archive, .. } => {
            let mode = Mode(fs::metadata(path)?.permissions().mode() & 0o7777);
            write(path, &archive.shipped(path)?, mode)
        }
        _ if path.is_dir() => Ok(fs::remove_dir_all(path)?),
        _ => Ok(fs::remove_file(path)?),
    };
    change().with_context(|| format!("changing {}", path.display()))
}

/// Written beside the file and renamed over it, so a crash never leaves half a sudoers file.
pub fn write(path: &Path, bytes: &[u8], mode: Mode) -> Result<()> {
    let dir = path.parent().context("no parent folder")?;
    fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(bytes)?;
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(mode.0))?;
    file.persist(path)?;
    Ok(())
}

pub fn link(path: &Path, target: &Path) -> Result<()> {
    let dir = path.parent().context("no parent folder")?;
    fs::create_dir_all(dir)?;
    let name = path.file_name().context("no file name")?.to_string_lossy();
    let temp = dir.join(format!(".{name}.mothership"));
    if fs::symlink_metadata(&temp).is_ok() {
        fs::remove_file(&temp)?;
    }
    unix::fs::symlink(target, &temp)?;
    fs::rename(&temp, path)?;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/revert.rs"]
mod tests;
