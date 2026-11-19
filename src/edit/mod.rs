mod doc;
mod etc;
mod home;

use doc::{declare, edit_toml, tool_entry, undeclare};

use crate::{
    file,
    fleet::{self, Fleet, Manager, Member, Tool},
    kind::Level,
    package::Distro,
    prompt::Prompt,
    secrets::Storage,
};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
};
use toml_edit::value;

#[derive(Clone, Copy)]
pub enum Key {
    Packages,
    BuiltPackages,
    Services,
    UserServices,
    Groups,
    Tools(Manager),
}

impl Key {
    fn path(self) -> (Option<&'static str>, &'static str) {
        match self {
            Key::Packages => (None, "packages"),
            Key::BuiltPackages => (None, "built_packages"),
            Key::Services => (None, "services"),
            Key::UserServices => (None, "user_services"),
            Key::Groups => (None, "groups"),
            Key::Tools(manager) => (Some("tools"), manager.name()),
        }
    }

    fn dotted(self) -> String {
        match self.path() {
            (Some(table), name) => format!("{table}.{name}"),
            (None, name) => name.into(),
        }
    }

    fn level(self) -> Level {
        match self {
            Key::Packages | Key::BuiltPackages | Key::Services => Level::Machine,
            Key::UserServices | Key::Groups | Key::Tools(_) => Level::User,
        }
    }

    fn file(self) -> &'static str {
        match self.level() {
            Level::Machine => fleet::SYSTEM_TOML,
            Level::User => fleet::USER_TOML,
        }
    }
}

#[derive(Clone)]
pub enum Add {
    ListPackage(String),
    UnlistPackage(String),
    Declare {
        key: Key,
        name: String,
    },
    DeclareTool {
        manager: Manager,
        tool: Tool,
    },
    Undeclare {
        key: Key,
        name: String,
    },
    UndeclareInstaller(String),
    Shell(String),
    Linger(bool),
    Track {
        path: PathBuf,
        storage: Storage,
    },
    Update {
        path: PathBuf,
        storage: Storage,
    },
    Forget(PathBuf),
    CopyEtc {
        rel: PathBuf,
        storage: Storage,
    },
    RemoveEtc(PathBuf),
    Pref {
        user_js: PathBuf,
        key: String,
        value: String,
    },
}

impl Add {
    pub fn encrypted(&self) -> Option<Add> {
        let mut add = self.clone();
        match &mut add {
            Add::Track { storage, .. }
            | Add::Update { storage, .. }
            | Add::CopyEtc { storage, .. } => {
                *storage = Storage::Encrypted;
                Some(add)
            }
            _ => None,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Add::ListPackage(_) => "add it to `packages` in system.toml".into(),
            Add::UnlistPackage(_) => "remove it from system.toml".into(),
            Add::Declare { key, .. } => {
                format!("add it to `{}` in {}", key.dotted(), key.file())
            }
            Add::DeclareTool { manager, .. } => {
                let path = Key::Tools(*manager).dotted();
                format!("write this machine's source into `{path}` in user.toml")
            }
            Add::Undeclare { key, .. } => {
                format!("remove it from `{}` in {}", key.dotted(), key.file())
            }
            Add::UndeclareInstaller(_) => "remove its installer from user.toml".into(),
            Add::Shell(shell) => format!("make {shell} the user's shell"),
            Add::Linger(true) => "set `linger = true` in user.toml".into(),
            Add::Linger(false) => "remove `linger` from user.toml".into(),
            Add::Track { storage, .. } => {
                format!("add it to chezmoi{}", encrypted_suffix(*storage))
            }
            Add::Update { storage, .. } => {
                format!(
                    "copy this machine's version into chezmoi{}",
                    encrypted_suffix(*storage)
                )
            }
            Add::CopyEtc { storage, .. } => {
                format!("copy it into the type's etc/{}", encrypted_suffix(*storage))
            }
            Add::Forget(_) => "remove it from chezmoi".into(),
            Add::RemoveEtc(_) => "remove it from the type's etc/".into(),
            Add::Pref { .. } => "write this machine's value into user.js".into(),
        }
    }

    pub fn apply(
        &self,
        fleet: &Fleet,
        member: Option<&Member>,
        prompt: &mut Prompt,
        seen: Option<&str>,
    ) -> Result<bool> {
        let member = || member.context("the item belongs to no user");
        match self {
            Add::ListPackage(name) => {
                declare(fleet, None, package_key(fleet, name)?, name, name.into())?
            }
            Add::UnlistPackage(name) => {
                undeclare(fleet, None, &[Key::Packages, Key::BuiltPackages], name)?
            }
            Add::Declare { key, name } => declare(fleet, member().ok(), *key, name, name.into())?,
            Add::DeclareTool { manager, tool } => declare(
                fleet,
                Some(member()?),
                Key::Tools(*manager),
                tool.name(),
                tool_entry(tool),
            )?,
            Add::Undeclare { key, name } => undeclare(fleet, member().ok(), &[*key], name)?,
            Add::UndeclareInstaller(name) => edit_toml(&member()?.path(), |doc| {
                let installers = doc["installers"]
                    .as_array_of_tables_mut()
                    .context("user.toml has no [[installers]]")?;
                let before = installers.len();
                installers.retain(|t| t.get("name").and_then(|n| n.as_str()) != Some(name));
                if installers.len() == before {
                    bail!("user.toml has no installer {name}");
                }
                Ok(())
            })?,
            Add::Shell(shell) => edit_toml(&member()?.path(), |doc| {
                doc["shell"] = value(shell.as_str());
                Ok(())
            })?,
            Add::Linger(linger) => edit_toml(&member()?.path(), |doc| {
                if *linger {
                    doc["linger"] = value(true);
                } else {
                    doc.remove("linger");
                }
                Ok(())
            })?,
            Add::Track { path, storage } => {
                return home::track(fleet, member()?, path, *storage, prompt, seen);
            }
            Add::Update { path, storage } => {
                return home::update(fleet, member()?, path, *storage, prompt, seen);
            }
            Add::Forget(path) => home::forget(fleet, member()?, path)?,
            Add::CopyEtc { rel, storage } => {
                etc::copy(fleet, rel, &Path::new(file::ETC).join(rel), *storage)?
            }
            Add::RemoveEtc(rel) => etc::remove(fleet, rel)?,
            Add::Pref {
                user_js,
                key,
                value,
            } => return home::pref(fleet, member()?, user_js, key, value, prompt),
        }
        Ok(true)
    }
}

fn encrypted_suffix(storage: Storage) -> &'static str {
    match storage {
        Storage::Clear => "",
        Storage::Encrypted => ", encrypted",
    }
}

fn append_line(path: &Path, line: &str) -> Result<()> {
    let mut text = file::read_if_any(path)?.unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(line);
    text.push('\n');
    fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

pub fn ignore(fleet: &Fleet, member: Option<&Member>, line: &str) -> Result<()> {
    let path = member.map_or_else(|| fleet.ignore_path(), Member::ignore_path);
    append_line(&path, one_line(line)?)
}

pub fn not_secret(fleet: &Fleet, member: Option<&Member>, subject: &str) -> Result<()> {
    let path = member.map_or_else(|| fleet.not_secret_path(), Member::not_secret_path);
    append_line(&path, one_line(&globset::escape(subject))?)
}

fn one_line(line: &str) -> Result<&str> {
    if line.chars().any(char::is_control) || line.trim() != line {
        bail!("its name holds a control character or spaces at an end: write a pattern by hand");
    }
    Ok(line)
}

fn package_key(fleet: &Fleet, name: &str) -> Result<Key> {
    let Some(why) = Distro::here()?.cannot_install(&fleet.aur(), name)? else {
        return Ok(Key::Packages);
    };
    println!("  {why}");
    println!("  It goes into `built_packages`.");
    Ok(Key::BuiltPackages)
}

#[cfg(test)]
#[path = "../../tests/edit.rs"]
mod tests;
