use super::{Change, Item, Kind, Ships};
use crate::{
    edit::{Add, Key},
    fleet::{Fleet, Member},
    package::Distro,
    preview::Preview,
    revert::Revert,
    systemd::{self, USER_DIRS},
    walk,
};
use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::CString,
    fs,
    path::{Path, PathBuf},
};

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
const DEBIAN_SYSTEM: &str = "/var/lib/systemd/deb-systemd-helper-enabled";
const DEBIAN_USER: &str = "/var/lib/systemd/deb-systemd-user-helper-enabled";

#[derive(Clone, Copy)]
enum Scope {
    System,
    User,
}

pub fn scan(fleet: &Fleet, ships: &Ships) -> Result<Vec<Item>> {
    let dirs = systemd::paths(&systemd::SYSTEM_DIRS);
    check(
        Unit {
            kind: Kind::Service,
            key: Key::Services,
            dirs: &dirs,
            own: &dirs[0],
            shared: None,
            defaults: &Defaults::load(ships.db.distro, Scope::System)?,
            member: None,
        },
        &fleet.system.services,
        ships,
    )
}

pub fn scan_user(member: &Member, ships: &Ships) -> Result<Vec<Item>> {
    let mut dirs = vec![member.user().home.join(".config/systemd/user")];
    dirs.extend(systemd::paths(&USER_DIRS));
    check(
        Unit {
            kind: Kind::UserService,
            key: Key::UserServices,
            dirs: &dirs,
            own: &dirs[0],
            shared: Some(&dirs[1]),
            defaults: &Defaults::load(ships.db.distro, Scope::User)?,
            member: Some(member),
        },
        &member.config.user_services,
        ships,
    )
}

pub fn global_defaults(distro: Distro) -> Result<Vec<PathBuf>> {
    let dirs = systemd::paths(&USER_DIRS);
    let defaults = Defaults::load(distro, Scope::User)?;
    let mut links = Vec::new();
    for enablement in systemd::enablements(&[&dirs[0]])? {
        if defaults.enables(&enablement.unit, &dirs)? {
            links.push(enablement.link);
        }
    }
    Ok(links)
}

struct Unit<'a> {
    kind: Kind,
    key: Key,
    dirs: &'a [PathBuf],
    own: &'a PathBuf,
    shared: Option<&'a PathBuf>,
    defaults: &'a Defaults,
    member: Option<&'a Member>,
}

fn check(scope: Unit, declared: &[String], ships: &Ships) -> Result<Vec<Item>> {
    let Unit {
        kind,
        key,
        dirs,
        own,
        shared,
        defaults,
        member,
    } = scope;
    let user = member.map(|m| m.user().name.clone());
    let enabled = systemd::enabled(&[own], dirs)?;
    let mut running = enabled.clone();
    if let Some(shared) = shared {
        running.extend(systemd::enabled(&[shared], dirs)?);
    }
    let declared: Vec<(&String, String)> = declared
        .iter()
        .map(|unit| Ok((unit, systemd::real_name(unit, dirs)?)))
        .collect::<Result<_>>()?;
    let real: Vec<String> = declared.iter().map(|(_, real)| real.clone()).collect();
    let covered = with_also(&real, dirs)?;
    let mut items = Vec::new();
    for unit in enabled.iter().filter(|unit| !covered.contains(*unit)) {
        if defaults.enables(unit, dirs)? {
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
        let file = systemd::unit_file(unit, dirs);
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
            .filter(|(_, real)| !running.contains(real))
            .map(|(unit, _)| {
                Item::new(kind, Change::Removed, *unit, "disabled")
                    .with(Add::Undeclare {
                        key,
                        name: unit.to_string(),
                    })
                    .reverting(Revert::Enable {
                        unit: unit.to_string(),
                        user: kind == Kind::UserService,
                    })
            }),
    );
    Ok(items)
}

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

fn also(unit: &str, unit_dirs: &[PathBuf]) -> Result<Vec<String>> {
    Ok(systemd::install_values(unit, unit_dirs, "Also")?
        .iter()
        .flat_map(|names| names.split_whitespace().map(str::to_owned))
        .collect())
}

enum Defaults {
    Presets(Presets),
    Packaged(BTreeSet<String>),
}

impl Defaults {
    fn load(distro: Distro, scope: Scope) -> Result<Self> {
        Ok(match (distro, scope) {
            (Distro::Arch, Scope::System) => Defaults::Presets(Presets::load(&SYSTEM_PRESETS)?),
            (Distro::Arch, Scope::User) => Defaults::Presets(Presets::load(&USER_PRESETS)?),
            (Distro::Debian, Scope::System) => {
                Defaults::Packaged(packaged_units(Path::new(DEBIAN_SYSTEM))?)
            }
            (Distro::Debian, Scope::User) => {
                Defaults::Packaged(packaged_units(Path::new(DEBIAN_USER))?)
            }
        })
    }

    fn enables(&self, unit: &str, unit_dirs: &[PathBuf]) -> Result<bool> {
        match self {
            Defaults::Presets(presets) => presets.enables(unit, unit_dirs),
            Defaults::Packaged(units) => Ok(units.contains(unit)),
        }
    }
}

fn packaged_units(dir: &Path) -> Result<BTreeSet<String>> {
    if !dir.exists() {
        return Ok(BTreeSet::new());
    }
    Ok(walk::files(dir)?
        .iter()
        .filter_map(|rel| rel.file_name())
        .map(|name| {
            let name = name.to_string_lossy();
            name.strip_suffix(".dsh-also")
                .map_or(name.to_string(), str::to_owned)
        })
        .collect())
}

struct Presets {
    rules: Vec<Rule>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Verb {
    Enable,
    Disable,
    Ignore,
}

struct Rule {
    verb: Verb,
    pattern: CString,
    instances: Vec<String>,
}

impl Rule {
    fn matches(&self, name: &str) -> bool {
        let Ok(name) = CString::new(name) else {
            return false;
        };
        unsafe { libc::fnmatch(self.pattern.as_ptr(), name.as_ptr(), libc::FNM_NOESCAPE) == 0 }
    }
}

impl Presets {
    fn load(dirs: &[&str]) -> Result<Self> {
        let mut files: BTreeMap<String, PathBuf> = BTreeMap::new();
        for dir in dirs.iter().rev() {
            for path in walk::children(Path::new(dir))? {
                if path.extension().is_some_and(|e| e == "preset")
                    && let Some(name) = path.file_name()
                {
                    files.insert(name.to_string_lossy().into_owned(), path);
                }
            }
        }
        let mut rules = Vec::new();
        for path in files.values() {
            let text =
                fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
            for (number, line) in text.lines().enumerate() {
                let at = || format!("{}:{}", path.display(), number + 1);
                let line = line.trim();
                if line.is_empty() || line.starts_with(['#', ';']) {
                    continue;
                }
                let mut words = line.split_whitespace();
                let verb = match words.next() {
                    Some("enable") => Verb::Enable,
                    Some("disable") => Verb::Disable,
                    Some("ignore") => Verb::Ignore,
                    _ => bail!("{}: expected enable, disable or ignore", at()),
                };
                let Some(pattern) = words.next() else {
                    bail!("{}: {line} names no unit", at());
                };
                rules.push(Rule {
                    verb,
                    pattern: CString::new(pattern).with_context(at)?,
                    instances: words.map(str::to_owned).collect(),
                });
            }
        }
        Ok(Presets { rules })
    }

    fn enables(&self, unit: &str, unit_dirs: &[PathBuf]) -> Result<bool> {
        let template = systemd::instance(unit);
        for rule in &self.rules {
            if rule.matches(unit) {
                return Ok(rule.verb == Verb::Enable);
            }
            let Some((template, instance)) = &template else {
                continue;
            };
            if !rule.matches(template) {
                continue;
            }
            let named = if rule.instances.is_empty() {
                systemd::install_values(template, unit_dirs, "DefaultInstance")?
                    .iter()
                    .any(|default| default == instance)
            } else {
                rule.instances.iter().any(|listed| listed == instance)
            };
            if named {
                return Ok(rule.verb == Verb::Enable);
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
#[path = "../../tests/services.rs"]
mod tests;
