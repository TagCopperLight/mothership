use crate::{
    cmd,
    fleet::{Fleet, Mode},
    pacman, user,
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
    WriteEtc(PathBuf),
    RestoreEtc {
        path: PathBuf,
        archive: PathBuf,
    },
    DeleteEtc(PathBuf),
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
            Revert::Install(_) => "install".into(),
            Revert::Uninstall(_) => "uninstall".into(),
            Revert::WriteEtc(_) | Revert::ApplyFile { .. } => "write the fleet's version".into(),
            Revert::RestoreEtc { .. } => "restore the package's version".into(),
            Revert::DeleteEtc(_) => "delete".into(),
            Revert::Shell(shell) => format!("make {shell} the login shell"),
            Revert::Join(_) => "join".into(),
            Revert::Leave(_) => "leave".into(),
            Revert::Enable { .. } => "enable and start".into(),
            Revert::Disable { .. } => "disable and stop".into(),
        }
    }
}

/// Packages go first, since the files and services after them may come from them. User units go
/// last, after chezmoi writes theirs.
pub fn run(fleet: &Fleet, reverts: &[&Revert]) -> Result<()> {
    let (mut install, mut uninstall, mut files) = (Vec::new(), Vec::new(), Vec::new());
    let (mut etc, mut account, mut system, mut user_units) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for revert in reverts.iter().copied() {
        match revert {
            Revert::Install(name) => install.push(name.as_str()),
            Revert::Uninstall(name) => uninstall.push(name.as_str()),
            Revert::WriteEtc(_) | Revert::RestoreEtc { .. } | Revert::DeleteEtc(_) => {
                etc.push(revert)
            }
            Revert::Shell(_) | Revert::Join(_) | Revert::Leave(_) => account.push(revert),
            Revert::Enable { user: false, .. } | Revert::Disable { user: false, .. } => {
                system.push(revert)
            }
            Revert::Enable { .. } | Revert::Disable { .. } => user_units.push(revert),
            Revert::ApplyFile { path, .. } => files.push(path.to_string_lossy()),
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
    let name = &user::current().name;
    for revert in account {
        match revert {
            Revert::Shell(shell) => cmd::root("usermod", &["--shell", shell, name])?,
            Revert::Join(group) => cmd::root("gpasswd", &["--add", name, group])?,
            Revert::Leave(group) => cmd::root("gpasswd", &["--delete", name, group])?,
            _ => unreachable!("sorted above"),
        }
    }
    for revert in system {
        systemctl(&[], revert)?;
    }
    if !files.is_empty() {
        let files: Vec<&str> = files.iter().map(|f| f.as_ref()).collect();
        cmd::run("chezmoi", &[&["apply", "--force"], &files[..]].concat())?;
    }
    if !user_units.is_empty() {
        let machine = format!("{name}@");
        let scope = ["--user", "--machine", &machine];
        cmd::root("systemctl", &[&scope[..], &["daemon-reload"]].concat())?;
        for revert in user_units {
            systemctl(&scope, revert)?;
        }
    }
    Ok(())
}

/// pacman installs what its repos have, and the fleet's AUR helper the rest, as the user.
fn install_packages(fleet: &Fleet, names: &[&str]) -> Result<()> {
    let in_repos: BTreeSet<String> = cmd::stdout("pacman", &["-Slq"])?
        .lines()
        .map(str::to_owned)
        .collect();
    let install = split_install(names, &in_repos, fleet.system.aur_helper.as_deref())?;
    if !install.repo.is_empty() {
        let args = [&["-S", "--needed", "--noconfirm"], &install.repo[..]].concat();
        cmd::root("pacman", &args)?;
    }
    if let Some((helper, aur)) = install.aur {
        cmd::run(helper, &[&["-S", "--needed"], &aur[..]].concat())?;
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
            let source = fleet.etc_dir().join(rel);
            if fs::symlink_metadata(&source)?.is_symlink() {
                link(path, &fs::read_link(&source)?)
            } else {
                write(path, &fs::read(&source)?, fleet.etc_mode(rel)?)
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
