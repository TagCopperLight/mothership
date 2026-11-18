use crate::{
    cmd::{self, As},
    fleet::{self, Fleet, Manager, Member, Mode, Tool},
    pacman, tools,
};
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    os::unix::{self, fs::PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};

/// How to make the machine match the fleet again, the other way from `edit::Add`.
pub enum Revert {
    Install(String),
    Uninstall(String),
    InstallTool {
        manager: Manager,
        tool: Tool,
    },
    /// The fleet moved it to another source.
    ReinstallTool {
        manager: Manager,
        tool: Tool,
    },
    UninstallTool {
        manager: Manager,
        name: String,
    },
    RunInstaller(String),
    WriteEtc(PathBuf),
    RestoreEtc {
        path: PathBuf,
        archive: PathBuf,
    },
    DeleteEtc(PathBuf),
    /// The machine lacks a user the type lists.
    CreateUser,
    /// The user's chezmoi doesn't read the fleet.
    InitChezmoi,
    Linger(bool),
    Shell(String),
    Join(String),
    Leave(String),
    Enable {
        unit: String,
        user: bool,
    },
    Disable {
        unit: String,
        user: bool,
    },
    /// `edited` when the file changed here since chezmoi last wrote it.
    ApplyFile {
        path: PathBuf,
        edited: bool,
    },
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

/// A revert, with the index of the type's user it belongs to.
pub type Owned<'a> = (Option<usize>, &'a Revert);

/// The machine goes first, then each user. Packages go first, since the rest may come from
/// them, and installers before tools, since bun comes from one. User units go last, after
/// chezmoi writes theirs.
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
    if !install.is_empty() {
        install_packages(fleet, &install)?;
    }
    if !uninstall.is_empty() {
        uninstall_packages(&uninstall)?;
    }
    for revert in &etc {
        change_etc(fleet, revert)?;
    }
    if !etc.is_empty() {
        cmd::root("systemctl", &["daemon-reload"])?;
    }
    for revert in system {
        systemctl(&[], revert)?;
    }
    Ok(())
}

/// A user the machine lacks has no other reverts until it exists.
fn run_member(member: &Member, reverts: &[&Revert]) -> Result<()> {
    if reverts.iter().any(|r| matches!(r, Revert::CreateUser)) {
        let mut args = vec!["--create-home"];
        if let Some(shell) = &member.config.shell {
            args.extend(["--shell", shell]);
        }
        args.push(&member.name);
        return cmd::root("useradd", &args);
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
        let machine = format!("{name}@");
        let scope = ["--user", "--machine", &machine];
        cmd::root("systemctl", &[&scope[..], &["daemon-reload"]].concat())?;
        for revert in units {
            systemctl(&scope, revert)?;
        }
    }
    Ok(())
}

/// pacman installs what its repos have, and the fleet's AUR helper the rest, as the user.
fn install_packages(fleet: &Fleet, names: &[&str]) -> Result<()> {
    let install = split_install(
        names,
        &pacman::in_repos()?,
        fleet.system.aur_helper.as_deref(),
    )?;
    if !install.repo.is_empty() {
        let args = [&["-S", "--needed", "--noconfirm"], &install.repo[..]].concat();
        cmd::root("pacman", &args)?;
    }
    // An AUR helper builds as a user, never as root.
    if let Some((helper, aur)) = install.aur {
        cmd::run(
            As::User(fleet.admin()),
            helper,
            &[&["-S", "--needed"], &aur[..]].concat(),
        )?;
    }
    Ok(())
}

#[derive(Debug, PartialEq)]
struct Install<'a> {
    repo: Vec<&'a str>,
    /// The helper, and what it installs.
    aur: Option<(&'a str, Vec<&'a str>)>,
}

/// Fails before anything is installed when AUR packages have no helper to install them.
fn split_install<'a>(
    names: &[&'a str],
    in_repos: &BTreeSet<String>,
    helper: Option<&'a str>,
) -> Result<Install<'a>> {
    let (repo, aur): (Vec<&str>, Vec<&str>) =
        names.iter().partition(|name| in_repos.contains(**name));
    if aur.is_empty() {
        return Ok(Install { repo, aur: None });
    }
    let Some(helper) = helper else {
        bail!(
            "{} not in pacman's repos, and system.toml sets no aur_helper",
            aur.join(", ")
        );
    };
    Ok(Install {
        repo,
        aur: Some((helper, aur)),
    })
}

/// A package another one needs stays, as a dependency.
fn uninstall_packages(names: &[&str]) -> Result<()> {
    cmd::root("pacman", &[&["-D", "--asdeps"], names].concat())?;
    let out = Command::new("pacman")
        .arg("-Qdtq")
        .output()
        .context("running pacman")?;
    // 1 means no package is left unneeded.
    if !matches!(out.status.code(), Some(0 | 1)) {
        bail!(
            "pacman -Qdtq failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let unneeded = String::from_utf8(out.stdout)?;
    let unneeded: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| unneeded.lines().any(|line| line == *name))
        .collect();
    if !unneeded.is_empty() {
        cmd::root("pacman", &[&["-Rs", "--noconfirm"], &unneeded[..]].concat())?;
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

fn change_etc(fleet: &Fleet, revert: &Revert) -> Result<()> {
    let (Revert::WriteEtc(path) | Revert::DeleteEtc(path) | Revert::RestoreEtc { path, .. }) =
        revert
    else {
        unreachable!("sorted above")
    };
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
            write(path, &pacman::shipped(archive, path)?, mode)
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
