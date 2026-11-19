use super::{Change, Item, Kind, Ships};
use crate::{
    edit::{Add, Key},
    fleet::{Fleet, Member},
    preview::Preview,
    revert::Revert,
};
use anyhow::{Context, Result};
use globset::Glob;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

const SYSTEM_PRESETS: [&str; 3] = [
    "/etc/systemd/system-preset",
    "/run/systemd/system-preset",
    "/usr/lib/systemd/system-preset",
];
/// A unit enabled through another lists its link in a folder named after that one.
const DEPENDENCY_DIRS: [&str; 3] = [".wants", ".requires", ".upholds"];
const USER_PRESETS: [&str; 3] = [
    "/etc/systemd/user-preset",
    "/run/systemd/user-preset",
    "/usr/lib/systemd/user-preset",
];

pub fn scan(fleet: &Fleet, ships: &Ships) -> Result<Vec<Item>> {
    let system = [
        PathBuf::from("/etc/systemd/system"),
        PathBuf::from("/run/systemd/system"),
        PathBuf::from("/usr/lib/systemd/system"),
    ];
    check(
        Unit {
            kind: Kind::Service,
            key: Key::Services,
            dirs: &system,
            enable_dirs: &[&system[0]],
            preset_dirs: &SYSTEM_PRESETS,
            member: None,
        },
        &fleet.system.services,
        ships,
    )
}

pub fn scan_user(member: &Member, ships: &Ships) -> Result<Vec<Item>> {
    let user = [
        member.user().home.join(".config/systemd/user"),
        PathBuf::from("/etc/systemd/user"),
        PathBuf::from("/run/systemd/user"),
        PathBuf::from("/usr/lib/systemd/user"),
    ];
    check(
        Unit {
            kind: Kind::UserService,
            key: Key::UserServices,
            dirs: &user,
            enable_dirs: &[&user[0], &user[1]],
            preset_dirs: &USER_PRESETS,
            member: Some(member),
        },
        &member.config.user_services,
        ships,
    )
}

struct Unit<'a> {
    kind: Kind,
    key: Key,
    dirs: &'a [PathBuf],
    enable_dirs: &'a [&'a PathBuf],
    preset_dirs: &'a [&'a str],
    member: Option<&'a Member>,
}

/// Units a package enables by default (its preset) don't count, nor the ones enabling a declared
/// unit brings along. `systemctl list-unit-files` says the same, but systemd takes half a second
/// to answer.
fn check(scope: Unit, declared: &[String], ships: &Ships) -> Result<Vec<Item>> {
    let Unit {
        kind,
        key,
        dirs: unit_dirs,
        enable_dirs,
        preset_dirs,
        member,
    } = scope;
    let user = member.map(|m| m.user().name.clone());
    let enabled = enabled(enable_dirs)?;
    let presets = Presets::load(preset_dirs)?;
    let covered = with_also(declared, unit_dirs)?;
    let mut items = Vec::new();
    for unit in enabled.iter().filter(|unit| !covered.contains(*unit)) {
        if presets.enables(unit, unit_dirs)? {
            continue;
        }
        let item = Item::new(kind, Change::Added, unit, "enabled")
            .reverting(Revert::Disable {
                unit: unit.clone(),
                user: kind == Kind::UserService,
            })
            .showing(Preview::Unit {
                name: unit.clone(),
                user: user.clone(),
            });
        let file = unit_file(unit, unit_dirs);
        items.push(match file {
            Some(file) if ships.path(&file, member)? => item.with(Add::Declare {
                key,
                name: unit.clone(),
            }),
            _ => item.because("its unit file comes from outside the fleet"),
        });
    }
    items.extend(
        declared
            .iter()
            .filter(|unit| !enabled.contains(*unit))
            .map(|unit| {
                Item::new(kind, Change::Removed, unit, "disabled")
                    .with(Add::Undeclare {
                        key,
                        name: unit.clone(),
                    })
                    .reverting(Revert::Enable {
                        unit: unit.clone(),
                        user: kind == Kind::UserService,
                    })
            }),
    );
    Ok(items)
}

/// `systemctl enable` also enables the units in `Also=`, and theirs in turn.
fn with_also(declared: &[String], unit_dirs: &[PathBuf]) -> Result<BTreeSet<String>> {
    let mut covered: BTreeSet<String> = declared.iter().cloned().collect();
    let mut todo = declared.to_vec();
    while let Some(unit) = todo.pop() {
        for other in also(&unit, unit_dirs)? {
            if covered.insert(other.clone()) {
                todo.push(other);
            }
        }
    }
    Ok(covered)
}

/// An instance has no file of its own: its template's applies.
fn unit_file(unit: &str, unit_dirs: &[PathBuf]) -> Option<PathBuf> {
    let template = instance(unit).map(|(template, _)| template);
    [Some(unit.to_owned()), template]
        .into_iter()
        .flatten()
        .flat_map(|name| unit_dirs.iter().map(move |d| d.join(&name)))
        .find(|p| p.exists())
}

fn also(unit: &str, unit_dirs: &[PathBuf]) -> Result<Vec<String>> {
    Ok(install_values(unit, unit_dirs, "Also")?
        .iter()
        .flat_map(|names| names.split_whitespace().map(str::to_owned))
        .collect())
}

/// The values of a key in the unit file's `[Install]` section.
fn install_values(unit: &str, unit_dirs: &[PathBuf], key: &str) -> Result<Vec<String>> {
    let Some(path) = unit_file(unit, unit_dirs) else {
        return Ok(Vec::new());
    };
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
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

/// `getty@tty1.service` is the instance `tty1` of the template `getty@.service`.
fn instance(unit: &str) -> Option<(String, &str)> {
    let (name, rest) = unit.split_once('@')?;
    let (instance, suffix) = rest.rsplit_once('.')?;
    (!instance.is_empty()).then(|| (format!("{name}@.{suffix}"), instance))
}

fn enabled(unit_dirs: &[&PathBuf]) -> Result<BTreeSet<String>> {
    let mut units = BTreeSet::new();
    for dir in unit_dirs {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => return Err(e).with_context(|| format!("reading {}", dir.display())),
        };
        for entry in entries {
            let path = entry?.path();
            if is_dependency_dir(&path) {
                // The link's name is the unit, which for a template names the instance.
                for link in fs::read_dir(&path)? {
                    let link = link?.path();
                    if target(&link)?.is_some() {
                        units.insert(file_name(&link));
                    }
                }
            } else if let Some(unit) = target(&path)? {
                // A link named like its target is `systemctl link`, not an alias.
                if path.file_name().is_some_and(|name| *name != *unit) {
                    units.insert(unit);
                }
            }
        }
    }
    Ok(units)
}

/// The unit a link enables. A link to /dev/null masks the unit, which the /etc check reports.
fn target(link: &Path) -> Result<Option<String>> {
    if !link.is_symlink() {
        return Ok(None);
    }
    let target = fs::read_link(link).with_context(|| format!("reading {}", link.display()))?;
    Ok((!is_mask(&target)).then(|| file_name(&target)))
}

fn is_mask(target: &Path) -> bool {
    target == Path::new("/dev/null")
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .expect("a unit path names a file")
        .to_string_lossy()
        .into_owned()
}

fn is_dependency_dir(path: &Path) -> bool {
    path.is_dir() && DEPENDENCY_DIRS.iter().any(|s| file_name(path).ends_with(s))
}

struct Presets {
    rules: Vec<Rule>,
}

struct Rule {
    enable: bool,
    glob: globset::GlobMatcher,
    /// The instances a rule on a template enables, after its name.
    instances: Vec<String>,
}

impl Presets {
    fn load(dirs: &[&str]) -> Result<Self> {
        let mut files: BTreeMap<String, PathBuf> = BTreeMap::new();
        for dir in dirs.iter().rev() {
            let entries = match fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(e) if e.kind() == ErrorKind::NotFound => continue,
                Err(e) => return Err(e).with_context(|| format!("reading {dir}")),
            };
            for entry in entries {
                let path = entry?.path();
                if path.extension().is_some_and(|e| e == "preset") {
                    let name = path.file_name().expect("has a name").to_string_lossy();
                    files.insert(name.into_owned(), path);
                }
            }
        }
        let mut rules = Vec::new();
        for path in files.values() {
            for line in fs::read_to_string(path)?.lines() {
                let mut words = line.split_whitespace();
                let enable = match words.next() {
                    Some("enable") => true,
                    Some("disable") => false,
                    _ => continue,
                };
                if let Some(pattern) = words.next() {
                    let glob = Glob::new(pattern)
                        .with_context(|| format!("{}: {pattern}", path.display()))?;
                    rules.push(Rule {
                        enable,
                        glob: glob.compile_matcher(),
                        instances: words.map(str::to_owned).collect(),
                    });
                }
            }
        }
        Ok(Presets { rules })
    }

    /// Like systemd, the first rule that names the unit decides. A rule on a template names
    /// the instances it lists, or the template's `DefaultInstance` when it lists none.
    fn enables(&self, unit: &str, unit_dirs: &[PathBuf]) -> Result<bool> {
        let template = instance(unit);
        for rule in &self.rules {
            if rule.glob.is_match(unit) {
                return Ok(rule.enable);
            }
            let Some((template, instance)) = &template else {
                continue;
            };
            if !rule.glob.is_match(template) {
                continue;
            }
            let named = if rule.instances.is_empty() {
                install_values(template, unit_dirs, "DefaultInstance")?
                    .iter()
                    .any(|default| default == instance)
            } else {
                rule.instances.iter().any(|listed| listed == instance)
            };
            if named {
                return Ok(rule.enable);
            }
        }
        Ok(true)
    }
}

pub fn is_enablement(path: &Path) -> bool {
    let in_unit_dir = path.parent().is_some_and(|dir| {
        dir == Path::new("/etc/systemd/system")
            || dir == Path::new("/etc/systemd/user")
            || dir.ends_with(".config/systemd/user")
    });
    let is_link = fs::read_link(path).is_ok_and(|target| !is_mask(&target));
    in_unit_dir && (DEPENDENCY_DIRS.iter().any(|s| file_name(path).ends_with(s)) || is_link)
}

#[cfg(test)]
#[path = "../../tests/services.rs"]
mod tests;
