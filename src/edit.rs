use crate::{
    chezmoi,
    fleet::{self, Fleet, Manager, Member, Mode, Table, Tool},
    homes,
    kind::Level,
    package::Distro,
    pref,
    prompt::Prompt,
    secrets::{self, Storage},
    snapshot::Snapshot,
    style, trust, walk,
};
use anyhow::{Context, Result, bail};
use std::{
    ffi::OsStr,
    fs,
    io::ErrorKind,
    os::unix::fs::PermissionsExt,
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
                let member = member()?;
                let root = member.chezmoi_root(fleet);
                let snapshot = taken(member, path, seen)?;
                if snapshot.path(member.user(), path)?.is_dir() {
                    let count = walk::files(&snapshot.path(member.user(), path)?)?.len();
                    let files = style::count(count, "file");
                    if !prompt.confirm(&format!("  That adds {files}. Go on"))? {
                        return Ok(false);
                    }
                }
                chezmoi_add(fleet, member, &root, &snapshot, path, *storage)?;
            }
            Add::Update { path, storage } => {
                let member = member()?;
                let root = member.chezmoi_root(fleet);
                let source = root.source_path(path)?;
                if source.extension().is_some_and(|e| e == "tmpl") {
                    bail!("{} is a template, edit it by hand", source.display());
                }
                let clear = !source
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("encrypted_"));
                let link = fs::symlink_metadata(&source)?.is_symlink();
                if link && *storage == Storage::Encrypted && clear {
                    bail!(
                        "{} is shared: encrypt it in {} by hand",
                        source.display(),
                        homes::target(&fleet.dir, &source)?.display()
                    );
                }
                if link && !confirm_shared(fleet, member, &source, prompt)? {
                    return Ok(false);
                }
                let snapshot = taken(member, path, seen)?;
                if link {
                    update_shared(fleet, &root, &snapshot, &source, path)?;
                } else if *storage == Storage::Clear {
                    let args = [OsStr::new("re-add"), path.as_os_str()];
                    root.run(&own_list(fleet, member)?, &snapshot, &args)?;
                } else {
                    chezmoi_add(fleet, member, &root, &snapshot, path, Storage::Encrypted)?;
                    if clear {
                        println!("  git history keeps the version in clear, so change it.");
                    }
                }
            }
            Add::Forget(path) => {
                let member = member()?;
                let root = member.chezmoi_root(fleet);
                let empty = Snapshot::take(member.user(), &[])?;
                let args = [
                    OsStr::new("forget"),
                    OsStr::new("--force"),
                    path.as_os_str(),
                ];
                root.run(&own_list(fleet, member)?, &empty, &args)?
            }
            Add::CopyEtc { rel, storage } => {
                copy_etc(fleet, rel, &Path::new(fleet::ETC).join(rel), *storage)?
            }
            Add::RemoveEtc(rel) => remove_etc(fleet, rel)?,
            Add::Pref {
                user_js,
                key,
                value,
            } => {
                let member = member()?;
                let source = member.chezmoi_root(fleet).source_path(user_js)?;
                let file = match fs::symlink_metadata(&source)?.is_symlink() {
                    true => {
                        if !confirm_shared(fleet, member, &source, prompt)? {
                            return Ok(false);
                        }
                        homes::target(&fleet.dir, &source)?
                    }
                    false => source,
                };
                set_pref(&file, key, value)?
            }
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

fn taken(member: &Member, path: &Path, seen: Option<&str>) -> Result<Snapshot> {
    let snapshot = Snapshot::take(member.user(), &[path])?;
    if seen.is_some_and(|seen| seen != snapshot.digest()) {
        bail!("it changed since it was shown: pass, then review it again");
    }
    Ok(snapshot)
}

fn confirm_shared(
    fleet: &Fleet,
    member: &Member,
    source: &Path,
    prompt: &mut Prompt,
) -> Result<bool> {
    let shared = homes::target(&fleet.dir, source)?;
    let others: Vec<String> = homes::users(&fleet.dir, source)?
        .into_iter()
        .filter(|user| *user != member.name)
        .collect();
    let shown = shared.strip_prefix(&fleet.dir).unwrap_or(&shared).display();
    println!(
        "  {shown} is shared: {}'s version reaches {} too.",
        member.name,
        others.join(" and ")
    );
    prompt.confirm("  Go on")
}

fn own_list(fleet: &Fleet, member: &Member) -> Result<String> {
    trust::home_list(&fleet.dir, &homes::source(&fleet.dir, &member.name))
}

fn update_shared(
    fleet: &Fleet,
    root: &chezmoi::Root,
    snapshot: &Snapshot,
    source: &Path,
    path: &Path,
) -> Result<()> {
    let shared = homes::target(&fleet.dir, source)?;
    let link = fs::read_link(source)?;
    let recipients = trust::home_list(&fleet.dir, source)?;
    root.run(
        &recipients,
        snapshot,
        &[OsStr::new("re-add"), path.as_os_str()],
    )?;
    fs::rename(source, &shared)?;
    walk::link(source, &link)
}

fn chezmoi_add(
    fleet: &Fleet,
    member: &Member,
    root: &chezmoi::Root,
    snapshot: &Snapshot,
    path: &Path,
    storage: Storage,
) -> Result<()> {
    let path = path.as_os_str();
    let recipients = own_list(fleet, member)?;
    if storage == Storage::Clear {
        return root.run(&recipients, snapshot, &[OsStr::new("add"), path]);
    }
    if !member.open {
        bail!(
            "{}'s key isn't in {}: `mothership trust` it first",
            member.name,
            member.recipients().display()
        );
    }
    root.run(
        &recipients,
        snapshot,
        &[OsStr::new("add"), OsStr::new("--encrypt"), path],
    )
}

fn append_line(path: &Path, line: &str) -> Result<()> {
    let mut text = walk::read_if_any(path)?.unwrap_or_default();
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
    let Some(why) = Distro::here()?.cannot_install(fleet, name)? else {
        return Ok(Key::Packages);
    };
    println!("  {why}");
    println!("  It goes into `built_packages`.");
    Ok(Key::BuiltPackages)
}

fn file(fleet: &Fleet, member: Option<&Member>, key: Key) -> Result<(PathBuf, Vec<Table>)> {
    match (key.level(), member) {
        (Level::Machine, _) => {
            let tables = [vec![Table::Top], fleet.blocks.clone()].concat();
            Ok((fleet.system_path(), tables))
        }
        (Level::User, Some(member)) => Ok((member.path(), vec![Table::Top])),
        (Level::User, None) => bail!("`{}` belongs to a user", key.dotted()),
    }
}

fn undeclare(fleet: &Fleet, member: Option<&Member>, keys: &[Key], name: &str) -> Result<()> {
    let (path, tables) = file(fleet, member, keys[0])?;
    edit_toml(&path, |doc| {
        let mut found = false;
        for table in &tables {
            for key in keys {
                let (parent, last) = key.path();
                let path: Vec<&str> = parent.into_iter().chain([last]).collect();
                if let Some(array) = find_array(doc, table, &path) {
                    found |= remove(array, name);
                }
            }
        }
        if !found {
            let keys: Vec<String> = keys.iter().map(|k| format!("`{}`", k.dotted())).collect();
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
    let (parent, name) = key.path();
    let mut table = doc.as_table_mut();
    if let Some(parent) = parent {
        table = table
            .entry(parent)
            .or_insert(toml_edit::table())
            .as_table_mut()
            .with_context(|| format!("`{parent}` in {} is not a table", key.file()))?;
    }
    table
        .entry(name)
        .or_insert(value(Array::new()))
        .as_array_mut()
        .with_context(|| format!("`{}` in {} is not an array", key.dotted(), key.file()))
}

fn find_array<'a>(doc: &'a mut DocumentMut, table: &Table, key: &[&str]) -> Option<&'a mut Array> {
    let mut item = doc.as_item_mut();
    for part in table.path().into_iter().chain(key.iter().copied()) {
        item = item.get_mut(part)?;
    }
    item.as_array_mut()
}

fn push(array: &mut Array, entry: Value) {
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

fn copy_etc(fleet: &Fleet, rel: &Path, path: &Path, storage: Storage) -> Result<()> {
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
    let (dest, other) = match storage {
        Storage::Clear => (clear.as_path(), sealed.as_path()),
        Storage::Encrypted => (sealed.as_path(), clear.as_path()),
    };
    fs::create_dir_all(dest.parent().expect("under etc/"))?;
    let meta = fs::symlink_metadata(path)?;
    if meta.is_symlink() {
        if storage == Storage::Encrypted {
            bail!("{} is a link, which holds no secret", path.display());
        }
        walk::link(dest, &fs::read_link(path)?)?;
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
        if storage == Storage::Encrypted {
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
            fs::Permissions::from_mode(Mode::default_for(executable, Storage::Clear).0),
        )?;
        let default = Mode::default_for(executable, storage);
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
    if walk::remove_if_found(other)? && storage == Storage::Encrypted {
        println!("  git history keeps the version in clear, so change it.");
    }
    Ok(())
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
        for table in &fleet.blocks {
            if let Some(array) = find_array(doc, table, &["etc"]) {
                remove(array, &rel);
            }
        }
        if let Some(modes) = doc.get_mut("etc_modes").and_then(|m| m.as_table_like_mut()) {
            modes.remove(&rel);
        }
        Ok(())
    })
}

fn set_pref(source: &Path, key: &str, value: &str) -> Result<()> {
    if source
        .extension()
        .is_some_and(|e| e == "age" || e == "tmpl")
    {
        bail!(
            "{} is encrypted or a template: set the pref in it by hand",
            source.display()
        );
    }
    let line = pref::line(key, value);
    let text = fs::read_to_string(source)?;
    let sets_it = |l: &str| pref::parse_line(l).is_some_and(|(k, _)| k == key);
    if !text.lines().any(sets_it) {
        return append_line(source, &line);
    }
    let out: Vec<&str> = text
        .lines()
        .map(|l| if sets_it(l) { line.as_str() } else { l })
        .collect();
    fs::write(source, out.join("\n") + "\n")
        .with_context(|| format!("writing {}", source.display()))
}

#[cfg(test)]
#[path = "../tests/edit.rs"]
mod tests;
