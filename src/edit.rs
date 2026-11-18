use crate::{
    cmd,
    fleet::{Fleet, Manager, Mode, Tool},
    pacman,
    prompt::Prompt,
    user, walk,
};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    io::ErrorKind,
    os::unix::{self, fs::PermissionsExt},
    path::{Path, PathBuf},
};
use toml_edit::{Array, DocumentMut, InlineTable, Value, value};

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
    fn path(self) -> Vec<&'static str> {
        match self {
            Key::Packages => vec!["packages"],
            Key::BuiltPackages => vec!["built_packages"],
            Key::Services => vec!["services"],
            Key::UserServices => vec!["user_services"],
            Key::Groups => vec!["groups"],
            Key::Tools(manager) => vec!["tools", manager.name()],
        }
    }
}

/// How to make the fleet accept what this machine has.
pub enum Add {
    ListPackage(String),
    UnlistPackage(String),
    Declare {
        key: Key,
        name: String,
    },
    /// Replaces the tool's entry when it has one.
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
    Track(PathBuf),
    Update(PathBuf),
    CopyEtc(PathBuf),
    RemoveEtc(PathBuf),
    Pref {
        user_js: PathBuf,
        key: String,
        value: String,
    },
}

impl Add {
    pub fn describe(&self) -> String {
        match self {
            Add::ListPackage(_) => "add it to `packages` in system.toml".into(),
            Add::UnlistPackage(_) => "remove it from system.toml".into(),
            Add::Declare { key, .. } => {
                format!("add it to `{}` in system.toml", key.path().join("."))
            }
            Add::DeclareTool { manager, .. } => {
                let path = Key::Tools(*manager).path().join(".");
                format!("write this machine's source into `{path}` in system.toml")
            }
            Add::Undeclare { key, .. } => {
                format!("remove it from `{}` in system.toml", key.path().join("."))
            }
            Add::UndeclareInstaller(_) => "remove its installer from system.toml".into(),
            Add::Shell(shell) => format!("make {shell} the type's shell"),
            Add::Track(_) => "add it to chezmoi".into(),
            Add::Update(_) => "copy this machine's version into chezmoi".into(),
            Add::CopyEtc(_) => "copy it into the type's etc/".into(),
            Add::RemoveEtc(_) => "remove it from the type's etc/".into(),
            Add::Pref { .. } => "write this machine's value into user.js".into(),
        }
    }

    /// Returns false when I cancel along the way.
    pub fn apply(&self, fleet: &Fleet, prompt: &mut Prompt) -> Result<bool> {
        match self {
            Add::ListPackage(name) => declare(fleet, package_key(fleet, name)?, name, name.into())?,
            Add::UnlistPackage(name) => {
                undeclare(fleet, &[Key::Packages, Key::BuiltPackages], name)?
            }
            Add::Declare { key, name } => declare(fleet, *key, name, name.into())?,
            Add::DeclareTool { manager, tool } => {
                declare(fleet, Key::Tools(*manager), tool.name(), tool_entry(tool))?
            }
            Add::Undeclare { key, name } => undeclare(fleet, &[*key], name)?,
            Add::UndeclareInstaller(name) => edit_system(fleet, |doc| {
                let installers = doc["installers"]
                    .as_array_of_tables_mut()
                    .context("system.toml has no [[installers]]")?;
                installers.retain(|t| t.get("name").and_then(|n| n.as_str()) != Some(name));
                Ok(())
            })?,
            Add::Shell(shell) => edit_system(fleet, |doc| {
                doc["shell"] = value(shell.as_str());
                Ok(())
            })?,
            Add::Track(path) => {
                if path.is_dir() {
                    let count = walk::files(path)?.len();
                    if !prompt.confirm(&format!("  That adds {count} files. Go on"))? {
                        return Ok(false);
                    }
                }
                cmd::run("chezmoi", &["add", &path.to_string_lossy()])?;
            }
            Add::Update(path) => {
                let source = source_path(path)?;
                if source.extension().is_some_and(|e| e == "tmpl") {
                    bail!("{} is a template, edit it by hand", source.display());
                }
                cmd::run("chezmoi", &["re-add", &path.to_string_lossy()])?;
            }
            Add::CopyEtc(path) => copy_etc(fleet, etc_rel(path), path)?,
            Add::RemoveEtc(path) => remove_etc(fleet, etc_rel(path))?,
            Add::Pref {
                user_js,
                key,
                value,
            } => set_pref(&source_path(user_js)?, key, value)?,
        }
        Ok(true)
    }
}

fn append_line(path: &Path, line: &str) -> Result<()> {
    let mut text =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(line);
    text.push('\n');
    fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

pub fn ignore(fleet: &Fleet, line: &str) -> Result<()> {
    append_line(&fleet.ignore_path(), line)
}

/// Apply installs a package from pacman's repos or the AUR. One found in neither was built
/// here, and only `built_packages`, which apply never installs, can hold it.
fn package_key(fleet: &Fleet, name: &str) -> Result<Key> {
    if pacman::in_repos()?.contains(name) {
        return Ok(Key::Packages);
    }
    let why = match fleet.system.aur_helper.as_deref() {
        None => "system.toml sets no aur_helper".to_owned(),
        Some(helper) => match cmd::stdout(helper, &["-Si", name]) {
            Ok(_) => return Ok(Key::Packages),
            Err(e) => format!("{e:#}"),
        },
    };
    println!("  It is in neither pacman's repos nor the AUR: {why}");
    println!("  It goes into `built_packages`.");
    Ok(Key::BuiltPackages)
}

/// The name may come from one of the machine's blocks.
fn undeclare(fleet: &Fleet, keys: &[Key], name: &str) -> Result<()> {
    edit_system(fleet, |doc| {
        let mut found = false;
        for table in &fleet.tables {
            for key in keys {
                if let Some(array) = find_array(doc, table, &key.path()) {
                    found |= remove(array, name);
                }
            }
        }
        if !found {
            let keys: Vec<String> = keys
                .iter()
                .map(|k| format!("`{}`", k.path().join(".")))
                .collect();
            bail!("{name} is in no {} of system.toml", keys.join(" or "));
        }
        Ok(())
    })
}

fn declare(fleet: &Fleet, key: Key, name: &str, entry: Value) -> Result<()> {
    edit_system(fleet, |doc| {
        let array = array(doc, key)?;
        if let Some(old) = array.iter_mut().find(|v| names(v, name)) {
            let decor = old.decor().clone();
            *old = entry;
            *old.decor_mut() = decor;
            return Ok(());
        }
        push(array, entry);
        Ok(())
    })
}

fn tool_entry(tool: &Tool) -> Value {
    let Tool::Git(tool) = tool else {
        return tool.name().into();
    };
    let mut table = InlineTable::new();
    table.insert("name", tool.name.as_str().into());
    table.insert("git", tool.git.as_str().into());
    if let Some(branch) = &tool.branch {
        table.insert("branch", branch.as_str().into());
    }
    table.into()
}

fn edit_system(fleet: &Fleet, change: impl FnOnce(&mut DocumentMut) -> Result<()>) -> Result<()> {
    let path = fleet.type_dir.join("system.toml");
    let mut doc: DocumentMut = fs::read_to_string(&path)?
        .parse()
        .with_context(|| format!("parsing {}", path.display()))?;
    change(&mut doc)?;
    fs::write(&path, doc.to_string()).with_context(|| format!("writing {}", path.display()))
}

fn array(doc: &mut DocumentMut, key: Key) -> Result<&mut Array> {
    let path = key.path();
    let (last, parents) = path.split_last().expect("paths are not empty");
    let mut table = doc.as_table_mut();
    for parent in parents {
        table = table
            .entry(parent)
            .or_insert(toml_edit::table())
            .as_table_mut()
            .with_context(|| format!("`{parent}` in system.toml is not a table"))?;
    }
    table
        .entry(last)
        .or_insert(value(Array::new()))
        .as_array_mut()
        .with_context(|| format!("`{}` in system.toml is not an array", key.path().join(".")))
}

fn find_array<'a>(
    doc: &'a mut DocumentMut,
    table: &[String],
    key: &[&str],
) -> Option<&'a mut Array> {
    let mut item = doc.as_item_mut();
    for part in table.iter().map(String::as_str).chain(key.iter().copied()) {
        item = item.get_mut(part)?;
    }
    item.as_array_mut()
}

/// toml_edit keeps the comment at the end of an item's line before the next item, or in the
/// array's trailing part after the last one. push and remove keep each comment on its line.
fn push(array: &mut Array, entry: Value) {
    // A one-line array has no newline to copy.
    let indent = array
        .iter()
        .last()
        .map(prefix)
        .and_then(|p| p.rfind('\n').map(|n| p[n..].to_owned()));
    array.push(entry);
    let Some(indent) = indent else {
        return;
    };
    let trailing = raw(array.trailing());
    let (line_end, rest) = trailing.split_at(trailing.find('\n').unwrap_or(trailing.len()));
    let pushed = array.iter_mut().last().expect("just pushed");
    pushed.decor_mut().set_prefix(format!("{line_end}{indent}"));
    array.set_trailing(rest);
}

fn remove(array: &mut Array, name: &str) -> bool {
    let Some(i) = array.iter().position(|v| names(v, name)) else {
        return false;
    };
    let own = prefix(array.get(i).expect("found"));
    let next = match array.get(i + 1) {
        Some(next) => prefix(next),
        None => raw(array.trailing()),
    };
    // Lines above the removed item stay, and its own line's comment goes with it.
    let joined = match next.split_once('\n') {
        None => own,
        Some((_, below)) => {
            let above = own.rfind('\n').map_or(own.as_str(), |n| &own[..=n]);
            format!("{above}{below}")
        }
    };
    array.remove(i);
    match array.get_mut(i) {
        Some(next) => next.decor_mut().set_prefix(joined),
        None => array.set_trailing(joined),
    }
    true
}

fn prefix(value: &Value) -> String {
    value
        .decor()
        .prefix()
        .and_then(|p| p.as_str())
        .unwrap_or_default()
        .to_owned()
}

fn raw(text: &toml_edit::RawString) -> String {
    text.as_str().unwrap_or_default().to_owned()
}

/// A tool is either its name or a table with its name and source.
fn names(entry: &Value, name: &str) -> bool {
    entry.as_str() == Some(name)
        || entry
            .as_inline_table()
            .and_then(|t| t.get("name"))
            .and_then(Value::as_str)
            == Some(name)
}

fn source_path(target: &Path) -> Result<PathBuf> {
    Ok(PathBuf::from(
        cmd::stdout("chezmoi", &["source-path", &target.to_string_lossy()])?.trim(),
    ))
}

fn etc_rel(path: &Path) -> &Path {
    path.strip_prefix("/etc")
        .expect("the /etc scan reports paths under /etc")
}

/// A mode git can't keep goes into `etc_modes`.
fn copy_etc(fleet: &Fleet, rel: &Path, path: &Path) -> Result<()> {
    if fleet.etc_elsewhere.contains(rel) {
        bail!(
            "etc/{} belongs to hardware or an option this machine doesn't have",
            rel.display()
        );
    }
    let dest = fleet.etc_dir().join(rel);
    let dest = dest.as_path();
    user::create_dir_all(dest.parent().expect("under etc/"))?;
    let meta = fs::symlink_metadata(path)?;
    if meta.is_symlink() {
        match fs::remove_file(dest) {
            Err(e) if e.kind() != ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        unix::fs::symlink(fs::read_link(path)?, dest)?;
        user::own(dest)?;
    } else if meta.is_dir() {
        bail!(
            "{} is a folder: copy the files you want one by one",
            path.display()
        );
    } else {
        let bytes = match fs::read(path) {
            Err(e) if e.kind() == ErrorKind::PermissionDenied => {
                bail!("{} needs root to read", path.display())
            }
            read => read?,
        };
        fs::write(dest, bytes)?;
        user::own(dest)?;
        let mode = meta.permissions().mode() & 0o7777;
        let default = Mode::default_for(mode & 0o111 != 0);
        fs::set_permissions(dest, fs::Permissions::from_mode(default.0))?;
        let rel = rel.to_string_lossy().into_owned();
        edit_system(fleet, |doc| {
            if Mode(mode) == default {
                if let Some(modes) = doc.get_mut("etc_modes").and_then(|m| m.as_table_like_mut()) {
                    modes.remove(&rel);
                }
                return Ok(());
            }
            doc.entry("etc_modes")
                .or_insert(toml_edit::table())
                .as_table_like_mut()
                .context("`etc_modes` in system.toml is not a table")?
                .insert(&rel, value(format!("{mode:o}")));
            Ok(())
        })?;
    }
    Ok(())
}

/// A block's file also leaves the block's `etc`.
fn remove_etc(fleet: &Fleet, rel: &Path) -> Result<()> {
    let dest = fleet.etc_dir().join(rel);
    fs::remove_file(&dest).with_context(|| format!("removing {}", dest.display()))?;
    let root = fleet.etc_dir();
    for dir in dest.ancestors().skip(1).take_while(|dir| *dir != root) {
        if fs::read_dir(dir)?.next().is_some() {
            break;
        }
        fs::remove_dir(dir)?;
    }
    let rel = rel.to_string_lossy();
    edit_system(fleet, |doc| {
        for table in &fleet.tables[1..] {
            if let Some(array) = find_array(doc, table, &["etc"]) {
                remove(array, &rel);
            }
        }
        Ok(())
    })
}

fn set_pref(source: &Path, key: &str, value: &str) -> Result<()> {
    let start = format!("user_pref(\"{key}\",");
    let line = format!("user_pref(\"{key}\", {value});");
    let text = fs::read_to_string(source)?;
    if text.lines().any(|l| l.trim_start().starts_with(&start)) {
        let out: Vec<&str> = text
            .lines()
            .map(|l| {
                if l.trim_start().starts_with(&start) {
                    line.as_str()
                } else {
                    l
                }
            })
            .collect();
        fs::write(source, out.join("\n") + "\n")?;
        Ok(())
    } else {
        append_line(source, &line)
    }
}

#[cfg(test)]
#[path = "../tests/edit.rs"]
mod tests;
