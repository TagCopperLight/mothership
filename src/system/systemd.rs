use crate::{util::file, util::walk};
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

pub const SYSTEM_DIRS: [&str; 3] = [
    "/etc/systemd/system",
    "/run/systemd/system",
    "/usr/lib/systemd/system",
];
pub const USER_DIRS: [&str; 3] = [
    "/etc/systemd/user",
    "/run/systemd/user",
    "/usr/lib/systemd/user",
];
pub const HOME_UNITS: &str = ".config/systemd/user";
const DEPENDENCY_DIRS: [&str; 3] = [".wants", ".requires", ".upholds"];
const MAX_ALIASES: usize = 16;

pub fn paths(dirs: &[&str]) -> Vec<PathBuf> {
    dirs.iter().map(PathBuf::from).collect()
}

pub struct Enablement {
    pub link: PathBuf,
    pub unit: String,
}

pub fn enablements(dirs: &[&PathBuf]) -> Result<Vec<Enablement>> {
    let mut found = Vec::new();
    for dir in dirs {
        for path in walk::children(dir)? {
            if is_dependency_dir(&path) {
                for link in walk::children(&path)? {
                    if target(&link)?.is_some()
                        && let Some(unit) = name(&link)
                    {
                        found.push(Enablement { link, unit });
                    }
                }
            } else if let Some(unit) = alias(&path)? {
                found.push(Enablement { link: path, unit });
            }
        }
    }
    Ok(found)
}

pub fn enabled(dirs: &[&PathBuf], unit_dirs: &[PathBuf]) -> Result<BTreeSet<String>> {
    enablements(dirs)?
        .into_iter()
        .map(|enablement| real_name(&enablement.unit, unit_dirs))
        .collect()
}

fn target(link: &Path) -> Result<Option<String>> {
    if !link.is_symlink() {
        return Ok(None);
    }
    let target = fs::read_link(link).with_context(|| format!("reading {}", link.display()))?;
    Ok(if is_mask(&target) {
        None
    } else {
        name(&target)
    })
}

fn alias(path: &Path) -> Result<Option<String>> {
    Ok(target(path)?.filter(|unit| name(path).as_ref() != Some(unit)))
}

fn is_mask(target: &Path) -> bool {
    target == Path::new("/dev/null")
}

fn name(path: &Path) -> Option<String> {
    Some(path.file_name()?.to_string_lossy().into_owned())
}

fn is_dependency_dir(path: &Path) -> bool {
    path.is_dir() && is_dependency_name(path)
}

fn is_dependency_name(path: &Path) -> bool {
    name(path).is_some_and(|name| DEPENDENCY_DIRS.iter().any(|s| name.ends_with(s)))
}

pub fn is_enablement(path: &Path) -> bool {
    let in_unit_dir = path
        .parent()
        .is_some_and(|dir| dir == Path::new(SYSTEM_DIRS[0]) || dir.ends_with(HOME_UNITS));
    let is_alias = fs::read_link(path).is_ok_and(|target| {
        !is_mask(&target)
            && target
                .file_name()
                .is_some_and(|t| Some(t) != path.file_name())
    });
    in_unit_dir && (is_dependency_name(path) || is_alias)
}

pub fn unit_file(unit: &str, unit_dirs: &[PathBuf]) -> Option<PathBuf> {
    let template = instance(unit).map(|(template, _)| template);
    [Some(unit.to_owned()), template]
        .into_iter()
        .flatten()
        .flat_map(|name| unit_dirs.iter().map(move |d| d.join(&name)))
        .find(|p| p.exists())
}

pub fn real_name(unit: &str, unit_dirs: &[PathBuf]) -> Result<String> {
    let mut name = unit.to_owned();
    for _ in 0..MAX_ALIASES {
        let Some(path) = first_present(&name, unit_dirs)? else {
            return Ok(name);
        };
        match alias(&path)? {
            Some(next) => name = next,
            None => return Ok(name),
        }
    }
    bail!("{unit} is an alias of an alias, {MAX_ALIASES} times over")
}

fn first_present(unit: &str, unit_dirs: &[PathBuf]) -> Result<Option<PathBuf>> {
    for dir in unit_dirs {
        let path = dir.join(unit);
        let found = file::if_found(fs::symlink_metadata(&path))
            .with_context(|| format!("reading {}", path.display()))?;
        if found.is_some() {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

pub fn instance(unit: &str) -> Option<(String, &str)> {
    let (name, rest) = unit.split_once('@')?;
    let (instance, suffix) = rest.rsplit_once('.')?;
    (!instance.is_empty()).then(|| (format!("{name}@.{suffix}"), instance))
}

pub fn install_values(unit: &str, unit_dirs: &[PathBuf], key: &str) -> Result<Vec<String>> {
    let Some(path) = unit_file(unit, unit_dirs) else {
        return Ok(Vec::new());
    };
    let bytes = file::read(&path)?.with_context(|| format!("{} vanished", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let mut install = false;
    let mut values = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            install = line == "[Install]";
        } else if install
            && let Some((name, value)) = line.split_once('=')
            && name.trim() == key
        {
            values.push(value.trim().to_owned());
        }
    }
    Ok(values)
}

#[cfg(test)]
#[path = "../../tests/system/systemd.rs"]
mod tests;
