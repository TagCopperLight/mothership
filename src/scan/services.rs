use super::{Item, Kind};
use crate::{
    edit::{Add, Key},
    fleet::{self, Fleet},
    preview::Preview,
};
use anyhow::{Context, Result};
use globset::Glob;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

/// A preset file in an earlier folder hides the file of the same name in a later one.
const SYSTEM_PRESETS: [&str; 3] = [
    "/etc/systemd/system-preset",
    "/run/systemd/system-preset",
    "/usr/lib/systemd/system-preset",
];
const USER_PRESETS: [&str; 3] = [
    "/etc/systemd/user-preset",
    "/run/systemd/user-preset",
    "/usr/lib/systemd/user-preset",
];

pub fn scan(fleet: &Fleet) -> Result<Vec<Item>> {
    let system = [PathBuf::from("/etc/systemd/system")];
    let user = [
        PathBuf::from("/etc/systemd/user"),
        fleet::home().join(".config/systemd/user"),
    ];
    let mut items = check(
        Kind::Service,
        Key::Services,
        &system,
        &SYSTEM_PRESETS,
        &fleet.system.services,
    )?;
    items.extend(check(
        Kind::UserService,
        Key::UserServices,
        &user,
        &USER_PRESETS,
        &fleet.system.user_services,
    )?);
    Ok(items)
}

/// Units a package enables by default (its preset) don't count. `systemctl list-unit-files`
/// says the same, but systemd takes half a second to answer.
fn check(
    kind: Kind,
    key: Key,
    unit_dirs: &[PathBuf],
    preset_dirs: &[&str],
    declared: &[String],
) -> Result<Vec<Item>> {
    let enabled = enabled(unit_dirs)?;
    let presets = Presets::load(preset_dirs)?;
    let mut items: Vec<Item> = enabled
        .iter()
        .filter(|unit| !presets.enables(unit) && !declared.contains(unit))
        .map(|unit| {
            Item::new(kind, unit, "enabled, not declared")
                .with(Add::Declare {
                    key,
                    name: unit.clone(),
                })
                .showing(Preview::Unit {
                    name: unit.clone(),
                    user: kind == Kind::UserService,
                })
        })
        .collect();
    items.extend(
        declared
            .iter()
            .filter(|unit| !enabled.contains(*unit))
            .map(|unit| {
                Item::new(kind, unit, "declared, not enabled").with(Add::Undeclare {
                    key,
                    name: unit.clone(),
                })
            }),
    );
    Ok(items)
}

/// A template's instances count as the template, the way systemd lists them.
fn enabled(unit_dirs: &[PathBuf]) -> Result<BTreeSet<String>> {
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
                for link in fs::read_dir(&path)? {
                    units.extend(target(&link?.path()));
                }
            } else if let Some(unit) = target(&path) {
                // A link named like its target is `systemctl link`, not an alias.
                if path.file_name().is_some_and(|name| *name != *unit) {
                    units.insert(unit);
                }
            }
        }
    }
    Ok(units)
}

/// Links to /dev/null mask a unit.
fn target(link: &Path) -> Option<String> {
    let target = fs::read_link(link).ok()?;
    (target != Path::new("/dev/null"))
        .then(|| target.file_name()?.to_str().map(str::to_owned))
        .flatten()
}

fn is_dependency_dir(path: &Path) -> bool {
    path.is_dir()
        && path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
            [".wants", ".requires", ".upholds"]
                .iter()
                .any(|s| n.ends_with(s))
        })
}

/// The first line that matches a unit decides, and a unit no line matches is enabled.
struct Presets {
    rules: Vec<(bool, globset::GlobMatcher)>,
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
                    rules.push((enable, glob.compile_matcher()));
                }
            }
        }
        Ok(Presets { rules })
    }

    fn enables(&self, unit: &str) -> bool {
        self.rules
            .iter()
            .find(|(_, glob)| glob.is_match(unit))
            .is_none_or(|(enable, _)| *enable)
    }
}

/// What `systemctl enable` creates: the services check reports those, not the file scans.
pub fn is_enablement(path: &Path) -> bool {
    let in_unit_dir = path.parent().is_some_and(|dir| {
        dir == Path::new("/etc/systemd/system")
            || dir == Path::new("/etc/systemd/user")
            || dir.ends_with(".config/systemd/user")
    });
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    in_unit_dir
        && ([".wants", ".requires", ".upholds"]
            .iter()
            .any(|s| name.ends_with(s))
            || path.is_symlink())
}
