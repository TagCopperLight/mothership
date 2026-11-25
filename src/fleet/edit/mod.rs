mod doc;
mod etc;
mod home;
pub mod secret;

use doc::{declare, edit_toml, tool_entry, undeclare};
pub use home::OwnerOnly;

use crate::{
    fleet::{self, Fleet, Manager, Member, Tool, homes},
    package::Distro,
    secrets::Storage,
    services,
    sync::kind::Level,
    sync::preview::Seen,
    util::file,
    util::prompt::Prompt,
};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
};
use toml_edit::value;

const FILE_MODE: u32 = 0o644;

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
    ListUser(String),
    WriteWorkflow,
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
            Add::ListUser(_) => "list the user in the type, with an empty home".into(),
            Add::WriteWorkflow => "write mothership's version into the fleet".into(),
        }
    }

    pub fn apply(
        &self,
        fleet: &Fleet,
        member: Option<&Member>,
        prompt: &mut Prompt,
        seen: &Seen,
    ) -> Result<Applied> {
        let member = || member.context("the item belongs to no user");
        match self {
            Add::ListPackage(name) => {
                require_installable(fleet, name)?;
                declare(fleet, None, Key::Packages, name, name.into())?
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
                etc::copy(fleet, rel, &Path::new(file::ETC).join(rel), *storage, seen)?
            }
            Add::RemoveEtc(rel) => etc::remove(fleet, rel)?,
            Add::Pref {
                user_js,
                key,
                value,
            } => return home::pref(fleet, member()?, user_js, key, value, prompt),
            Add::ListUser(name) => {
                let question = format!(
                    "  mothership then manages {name}'s account, services and files here. Go on"
                );
                if !prompt.confirm(&question)? {
                    return Ok(Applied::Declined);
                }
                list_user(fleet, name)?
            }
            Add::WriteWorkflow => file::write(
                &fleet.dir.join(services::WORKFLOW_PATH),
                services::WORKFLOW.as_bytes(),
                FILE_MODE,
            )?,
        }
        Ok(Applied::Added)
    }
}

pub enum Applied {
    Declined,
    Added,
    AddedThen(OwnerOnly),
}

fn list_user(fleet: &Fleet, name: &str) -> Result<()> {
    let dir = fleet.type_dir.join(fleet::USERS).join(name);
    if dir.exists() {
        bail!("{} is there already", dir.display());
    }
    let home = dir.join(homes::HOME);
    fs::create_dir_all(&home).with_context(|| format!("creating {}", home.display()))?;
    for empty in [dir.join(fleet::USER_TOML), home.join(homes::KEEP)] {
        file::write(&empty, b"", FILE_MODE)?;
    }
    Ok(())
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
    file::write(path, text.as_bytes(), FILE_MODE)
}

pub fn ignore(fleet: &Fleet, member: Option<&Member>, line: &str) -> Result<()> {
    let path = member.map_or_else(|| fleet.ignore_path(), |member| member.ignore_path());
    append_line(&path, one_line(line)?)
}

fn one_line(line: &str) -> Result<&str> {
    if line.chars().any(char::is_control) || line.trim() != line {
        bail!("its name holds a control character or spaces at an end: write a pattern by hand");
    }
    Ok(line)
}

fn require_installable(fleet: &Fleet, name: &str) -> Result<()> {
    match Distro::here()?.cannot_install(&fleet.aur(), name)? {
        None => Ok(()),
        Some(why) => {
            bail!("{why}. List it in `built_packages` by hand, with where it comes from")
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/fleet/edit/mod.rs"]
mod tests;
