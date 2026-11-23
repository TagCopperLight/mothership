use super::{Archive, Aur, Backup, Distro, Origin, Package, aur};
use crate::{
    util::cmd::{self, As},
    util::parallel,
};
use anyhow::{Context, Result, bail};
use std::{collections::BTreeSet, fs, path::Path};

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
    dirs.sort();
    parallel::map(&dirs, |dir| package(dir))
}

fn package(dir: &Path) -> Result<Package> {
    let read = |name| {
        let path = dir.join(name);
        fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))
    };
    let desc = read("desc")?;
    let files = read("files")?;
    parse(&desc, &files).with_context(|| format!("reading {}", dir.display()))
}

fn parse(desc: &str, files: &str) -> Result<Package> {
    let field = |name| {
        section(desc, name)
            .next()
            .with_context(|| format!("desc has no %{name}%"))
    };
    let name = field("NAME")?.to_owned();
    let archive = format!(
        "{name}-{}-{}.pkg.tar.zst",
        field("VERSION")?,
        field("ARCH")?
    );
    let backups = section(files, "BACKUP")
        .map(|line| {
            let (path, md5) = line
                .split_once('\t')
                .with_context(|| format!("a %BACKUP% line has no hash: {line}"))?;
            Ok(Backup {
                path: Path::new("/").join(path),
                md5: md5.to_owned(),
                origin: Origin::Archive,
            })
        })
        .collect::<Result<_>>()?;
    Ok(Package {
        description: section(desc, "DESC").next().unwrap_or_default().to_owned(),
        explicit: section(desc, "REASON").next() != Some("1"),
        required: false,
        files: section(files, "FILES").map(|f| format!("/{f}")).collect(),
        depends: names(desc, "DEPENDS"),
        provides: names(desc, "PROVIDES"),
        groups: Vec::new(),
        backups,
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

pub fn cannot_install(aur: &Aur, name: &str) -> Result<Option<String>> {
    if in_repos()?.contains(name) {
        return Ok(None);
    }
    let why = match aur.helper {
        None => "system.toml sets no aur_helper".to_owned(),
        Some(helper) => match cmd::stdout(As::User(aur.admin), helper.command(), &["-Si", name]) {
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

pub fn bootstrap(names: &[&str]) -> Result<()> {
    cmd::root(
        "pacman",
        &[&["-S", "--needed", "--noconfirm"], names].concat(),
    )
}

pub fn install(aur: &Aur, names: &[&str]) -> Result<()> {
    let install = split_install(names, &in_repos()?, aur.helper)?;
    if let Some((helper, _)) = &install.aur
        && !helper.installed(aur.admin)
    {
        bail!(
            "system.toml names {helper} as the AUR helper, but {} has no {helper}",
            aur.admin.name
        );
    }
    if !install.repo.is_empty() {
        let args = [&["-S", "--needed", "--noconfirm"], &install.repo[..]].concat();
        cmd::root("pacman", &args)?;
    }
    if let Some((helper, names)) = &install.aur {
        let _sudo = aur::NoPassword::grant(aur.admin)?;
        cmd::run(
            As::User(aur.admin),
            helper.command(),
            &[&["-S", "--needed", "--noconfirm"], &names[..]].concat(),
        )?;
    }
    Ok(())
}

#[derive(Debug, PartialEq)]
struct Install<'a> {
    repo: Vec<&'a str>,
    aur: Option<(aur::Helper, Vec<&'a str>)>,
}

fn split_install<'a>(
    names: &[&'a str],
    in_repos: &BTreeSet<String>,
    helper: Option<aur::Helper>,
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

pub fn uninstall(keep: &[&str], names: &[&str]) -> Result<BTreeSet<String>> {
    if !keep.is_empty() {
        cmd::root("pacman", &[&["-D", "--asexplicit"], keep].concat())?;
    }
    cmd::root("pacman", &[&["-D", "--asdeps"], names].concat())?;
    super::or_restore(remove_unneeded(names), || {
        cmd::root("pacman", &[&["-D", "--asexplicit"], names].concat())
    })
}

fn remove_unneeded(names: &[&str]) -> Result<BTreeSet<String>> {
    let unneeded = String::from_utf8(cmd::bytes_allowing_exit(As::Root, "pacman", &["-Qdtq"], 1)?)?;
    let unneeded: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| unneeded.lines().any(|line| line == *name))
        .collect();
    if !unneeded.is_empty() {
        cmd::root("pacman", &[&["-Rs", "--noconfirm"], &unneeded[..]].concat())?;
    }
    Ok(unneeded.into_iter().map(str::to_owned).collect())
}

#[cfg(test)]
#[path = "../../tests/package/pacman.rs"]
mod tests;
