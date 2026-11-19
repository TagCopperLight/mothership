use super::{Archive, Backup, Distro, Origin, Package, in_parallel};
use crate::{
    cmd::{self, As},
    fleet::Fleet,
};
use anyhow::{Context, Result, bail};
use std::{collections::BTreeSet, fs, path::Path, process::Command};

const LOCAL: &str = "/var/lib/pacman/local";
const CACHE: &str = "/var/cache/pacman/pkg";

pub fn load() -> Result<Vec<Package>> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(LOCAL).with_context(|| format!("reading {LOCAL}"))? {
        let dir = entry?.path();
        if dir.is_dir() {
            dirs.push(dir);
        }
    }
    // The folder's order depends on the filesystem, and the scan reports in this order.
    dirs.sort();
    in_parallel(&dirs, |dir| package(dir))
}

fn package(dir: &Path) -> Result<Package> {
    let read = |name| {
        let path = dir.join(name);
        fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))
    };
    let desc = read("desc")?;
    let files = read("files")?;
    let field = |name| section(&desc, name).next().unwrap_or_default().to_owned();
    let name = field("NAME");
    let archive = format!("{name}-{}-{}.pkg.tar.zst", field("VERSION"), field("ARCH"));
    Ok(Package {
        description: field("DESC"),
        // pacman writes REASON only for dependencies.
        explicit: section(&desc, "REASON").next() != Some("1"),
        files: section(&files, "FILES").map(|f| format!("/{f}")).collect(),
        depends: names(&desc, "DEPENDS"),
        provides: names(&desc, "PROVIDES"),
        backups: section(&files, "BACKUP")
            .filter_map(|line| line.split_once('\t'))
            .map(|(path, md5)| Backup {
                path: Path::new("/").join(path),
                md5: md5.to_owned(),
                origin: Origin::Archive,
            })
            .collect(),
        archive: Archive {
            path: Path::new(CACHE).join(archive),
            distro: Distro::Arch,
        },
        name,
    })
}

fn names(desc: &str, field: &str) -> Vec<String> {
    section(desc, field)
        .filter_map(|dep| dep.split(['<', '>', '=']).next())
        .map(str::to_owned)
        .collect()
}

fn section<'a>(text: &'a str, name: &str) -> impl Iterator<Item = &'a str> {
    let header = format!("%{name}%");
    text.lines()
        .skip_while(move |line| *line != header)
        .skip(1)
        .take_while(|line| !line.is_empty())
}

fn in_repos() -> Result<BTreeSet<String>> {
    Ok(cmd::stdout(As::Root, "pacman", &["-Slq"])?
        .lines()
        .map(str::to_owned)
        .collect())
}

pub fn cannot_install(fleet: &Fleet, name: &str) -> Result<Option<String>> {
    if in_repos()?.contains(name) {
        return Ok(None);
    }
    let why = match fleet.system.aur_helper.as_deref() {
        None => "system.toml sets no aur_helper".to_owned(),
        Some(helper) => match cmd::stdout(As::User(fleet.admin()), helper, &["-Si", name]) {
            Ok(_) => return Ok(None),
            Err(e) => format!("{e:#}"),
        },
    };
    Ok(Some(format!(
        "It is in neither pacman's repos nor the AUR: {why}"
    )))
}

pub fn shipped(archive: &Path, member: &Path) -> Result<Vec<u8>> {
    cmd::bytes(
        As::Root,
        "bsdtar",
        &[
            "-xOf",
            &archive.to_string_lossy(),
            &member.to_string_lossy(),
        ],
    )
}

pub fn install(fleet: &Fleet, names: &[&str]) -> Result<()> {
    let install = split_install(names, &in_repos()?, fleet.system.aur_helper.as_deref())?;
    if let Some((helper, _)) = &install.aur
        && !cmd::found(As::User(fleet.admin()), helper)
    {
        bail!(
            "system.toml names {helper} as the AUR helper, but {} has no {helper}",
            fleet.admin().name
        );
    }
    if !install.repo.is_empty() {
        let args = [&["-S", "--needed", "--noconfirm"], &install.repo[..]].concat();
        cmd::root("pacman", &args)?;
    }
    // An AUR helper builds as a user, never as root.
    if let Some((helper, aur)) = &install.aur {
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
    aur: Option<(&'a str, Vec<&'a str>)>,
}

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

pub fn uninstall(names: &[&str]) -> Result<()> {
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

#[cfg(test)]
#[path = "../../tests/pacman.rs"]
mod tests;
