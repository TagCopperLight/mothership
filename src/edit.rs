use crate::{
    chezmoi,
    cmd::{self, As},
    fleet::{Fleet, Manager, Member, Mode, Tool},
    ignore::Level,
    pacman,
    prompt::Prompt,
    secrets, walk,
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

    fn level(self) -> Level {
        match self {
            Key::Packages | Key::BuiltPackages | Key::Services => Level::Machine,
            Key::UserServices | Key::Groups | Key::Tools(_) => Level::User,
        }
    }

    fn file(self) -> &'static str {
        match self.level() {
            Level::Machine => "system.toml",
            Level::User => "user.toml",
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
        encrypt: bool,
    },
    Update {
        path: PathBuf,
        encrypt: bool,
    },
    CopyEtc {
        path: PathBuf,
        encrypt: bool,
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
            Add::Track { encrypt, .. }
            | Add::Update { encrypt, .. }
            | Add::CopyEtc { encrypt, .. } => {
                *encrypt = true;
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
                format!("add it to `{}` in {}", key.path().join("."), key.file())
            }
            Add::DeclareTool { manager, .. } => {
                let path = Key::Tools(*manager).path().join(".");
                format!("write this machine's source into `{path}` in user.toml")
            }
            Add::Undeclare { key, .. } => {
                format!(
                    "remove it from `{}` in {}",
                    key.path().join("."),
                    key.file()
                )
            }
            Add::UndeclareInstaller(_) => "remove its installer from user.toml".into(),
            Add::Shell(shell) => format!("make {shell} the user's shell"),
            Add::Linger(true) => "set `linger = true` in user.toml".into(),
            Add::Linger(false) => "remove `linger` from user.toml".into(),
            Add::Track { encrypt, .. } => {
                format!("add it to chezmoi{}", encrypted_suffix(*encrypt))
            }
            Add::Update { encrypt, .. } => {
                format!(
                    "copy this machine's version into chezmoi{}",
                    encrypted_suffix(*encrypt)
                )
            }
            Add::CopyEtc { encrypt, .. } => {
                format!("copy it into the type's etc/{}", encrypted_suffix(*encrypt))
            }
            Add::RemoveEtc(_) => "remove it from the type's etc/".into(),
            Add::Pref { .. } => "write this machine's value into user.js".into(),
        }
    }

    pub fn apply(
        &self,
        fleet: &Fleet,
        member: Option<&Member>,
        prompt: &mut Prompt,
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
                installers.retain(|t| t.get("name").and_then(|n| n.as_str()) != Some(name));
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
            Add::Track { path, encrypt } => {
                if path.is_dir() {
                    let count = walk::files(path)?.len();
                    if !prompt.confirm(&format!("  That adds {count} files. Go on"))? {
                        return Ok(false);
                    }
                }
                chezmoi_add(member()?, path, *encrypt)?;
            }
            Add::Update { path, encrypt } => {
                let member = member()?;
                let source = chezmoi::source_path(member.user(), path)?;
                if source.extension().is_some_and(|e| e == "tmpl") {
                    bail!("{} is a template, edit it by hand", source.display());
                }
                let clear = !source
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("encrypted_"));
                if !encrypt {
                    chezmoi::write_for(member.user(), &["re-add", &path.to_string_lossy()])?;
                } else {
                    chezmoi_add(member, path, true)?;
                    if clear {
                        println!("  git history keeps the version in clear, so change it.");
                    }
                }
            }
            Add::CopyEtc { path, encrypt } => copy_etc(fleet, etc_rel(path), path, *encrypt)?,
            Add::RemoveEtc(path) => remove_etc(fleet, etc_rel(path))?,
            Add::Pref {
                user_js,
                key,
                value,
            } => set_pref(
                &chezmoi::source_path(member()?.user(), user_js)?,
                key,
                value,
            )?,
        }
        Ok(true)
    }
}

fn encrypted_suffix(encrypt: bool) -> &'static str {
    if encrypt { ", encrypted" } else { "" }
}

/// chezmoi encrypts for the user's recipients, and turns a file it has in clear into an
/// encrypted one. The user's key must be among them, or the user couldn't open it.
fn chezmoi_add(member: &Member, path: &Path, encrypt: bool) -> Result<()> {
    let path = path.to_string_lossy();
    if !encrypt {
        return chezmoi::write_for(member.user(), &["add", &path]);
    }
    if !member.open {
        bail!(
            "{}'s key isn't in {}: `mothership trust` it first",
            member.name,
            member.recipients().display()
        );
    }
    chezmoi::write_for(member.user(), &["add", "--encrypt", &path])
}

fn append_line(path: &Path, line: &str) -> Result<()> {
    let mut text = match fs::read_to_string(path) {
        Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
        text => text.with_context(|| format!("reading {}", path.display()))?,
    };
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(line);
    text.push('\n');
    fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

pub fn ignore(fleet: &Fleet, member: Option<&Member>, line: &str) -> Result<()> {
    let path = member.map_or_else(|| fleet.ignore_path(), Member::ignore_path);
    append_line(&path, line)
}

pub fn not_secret(fleet: &Fleet, member: Option<&Member>, subject: &str) -> Result<()> {
    let path = member.map_or_else(|| fleet.not_secret_path(), Member::not_secret_path);
    append_line(&path, &globset::escape(subject))
}

/// Apply installs a package from pacman's repos or the AUR. One found in neither was built
/// here, and only `built_packages`, which apply never installs, can hold it.
fn package_key(fleet: &Fleet, name: &str) -> Result<Key> {
    if pacman::in_repos()?.contains(name) {
        return Ok(Key::Packages);
    }
    let why = match fleet.system.aur_helper.as_deref() {
        None => "system.toml sets no aur_helper".to_owned(),
        Some(helper) => match cmd::stdout(As::User(fleet.admin()), helper, &["-Si", name]) {
            Ok(_) => return Ok(Key::Packages),
            Err(e) => format!("{e:#}"),
        },
    };
    println!("  It is in neither pacman's repos nor the AUR: {why}");
    println!("  It goes into `built_packages`.");
    Ok(Key::BuiltPackages)
}

fn file(fleet: &Fleet, member: Option<&Member>, key: Key) -> Result<(PathBuf, Vec<Vec<String>>)> {
    match (key.level(), member) {
        (Level::Machine, _) => Ok((fleet.system_path(), fleet.tables.clone())),
        (Level::User, Some(member)) => Ok((member.path(), vec![vec![]])),
        (Level::User, None) => bail!("`{}` belongs to a user", key.path().join(".")),
    }
}

fn undeclare(fleet: &Fleet, member: Option<&Member>, keys: &[Key], name: &str) -> Result<()> {
    let (path, tables) = file(fleet, member, keys[0])?;
    edit_toml(&path, |doc| {
        let mut found = false;
        for table in &tables {
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
            bail!(
                "{name} is in no {} of {}",
                keys.join(" or "),
                path.display()
            );
        }
        Ok(())
    })
}

fn declare(
    fleet: &Fleet,
    member: Option<&Member>,
    key: Key,
    name: &str,
    entry: Value,
) -> Result<()> {
    let (path, _) = file(fleet, member, key)?;
    edit_toml(&path, |doc| {
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

fn edit_toml(path: &Path, change: impl FnOnce(&mut DocumentMut) -> Result<()>) -> Result<()> {
    let mut doc: DocumentMut = fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?
        .parse()
        .with_context(|| format!("parsing {}", path.display()))?;
    change(&mut doc)?;
    fs::write(path, doc.to_string()).with_context(|| format!("writing {}", path.display()))
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
            .with_context(|| format!("`{parent}` in {} is not a table", key.file()))?;
    }
    table
        .entry(last)
        .or_insert(value(Array::new()))
        .as_array_mut()
        .with_context(|| {
            format!(
                "`{}` in {} is not an array",
                key.path().join("."),
                key.file()
            )
        })
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

fn names(entry: &Value, name: &str) -> bool {
    entry.as_str() == Some(name)
        || entry
            .as_inline_table()
            .and_then(|t| t.get("name"))
            .and_then(Value::as_str)
            == Some(name)
}

fn etc_rel(path: &Path) -> &Path {
    path.strip_prefix("/etc")
        .expect("the /etc scan reports paths under /etc")
}

fn copy_etc(fleet: &Fleet, rel: &Path, path: &Path, encrypt: bool) -> Result<()> {
    if fleet.etc_elsewhere.contains(rel) {
        bail!(
            "etc/{} belongs to hardware or an option this machine doesn't have",
            rel.display()
        );
    }
    let clear = fleet.etc_dir().join(rel);
    let mut sealed = clear.clone().into_os_string();
    sealed.push(secrets::AGE);
    let sealed = PathBuf::from(sealed);
    let (dest, other) = if encrypt {
        (sealed.as_path(), clear.as_path())
    } else {
        (clear.as_path(), sealed.as_path())
    };
    fs::create_dir_all(dest.parent().expect("under etc/"))?;
    let meta = fs::symlink_metadata(path)?;
    if meta.is_symlink() {
        if encrypt {
            bail!("{} is a link, which holds no secret", path.display());
        }
        match fs::remove_file(dest) {
            Err(e) if e.kind() != ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        unix::fs::symlink(fs::read_link(path)?, dest)?;
    } else if meta.is_dir() {
        bail!(
            "{} is a folder: copy the files you want one by one",
            path.display()
        );
    } else {
        let mut bytes = match fs::read(path) {
            Err(e) if e.kind() == ErrorKind::PermissionDenied => {
                bail!("{} needs root to read", path.display())
            }
            read => read?,
        };
        if encrypt {
            if !fleet.etc_open() {
                bail!(
                    "root's key isn't in {}: `mothership trust` it first",
                    fleet.recipients().display()
                );
            }
            bytes = secrets::encrypt(&fleet.recipients(), &bytes)?;
        }
        fs::write(dest, bytes)?;
        let mode = meta.permissions().mode() & 0o7777;
        let executable = mode & 0o111 != 0;
        fs::set_permissions(
            dest,
            fs::Permissions::from_mode(Mode::default_for(executable, false).0),
        )?;
        let default = Mode::default_for(executable, encrypt);
        let rel = rel.to_string_lossy().into_owned();
        edit_toml(&fleet.system_path(), |doc| {
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
    match fs::remove_file(other) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

fn remove_etc(fleet: &Fleet, rel: &Path) -> Result<()> {
    let dest = fleet.etc_source(rel);
    fs::remove_file(&dest).with_context(|| format!("removing {}", dest.display()))?;
    let root = fleet.etc_dir();
    for dir in dest.ancestors().skip(1).take_while(|dir| *dir != root) {
        if fs::read_dir(dir)?.next().is_some() {
            break;
        }
        fs::remove_dir(dir)?;
    }
    let rel = rel.to_string_lossy();
    edit_toml(&fleet.system_path(), |doc| {
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
