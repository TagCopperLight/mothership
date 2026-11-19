use super::{FILE_MODE, Key};
use crate::{
    file,
    fleet::{Fleet, Member, Table, Tool},
    kind::Level,
};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
};
use toml_edit::{Array, DocumentMut, InlineTable, Value, value};

pub fn file(fleet: &Fleet, member: Option<&Member>, key: Key) -> Result<(PathBuf, Vec<Table>)> {
    match (key.level(), member) {
        (Level::Machine, _) => {
            let tables = [vec![Table::Top], fleet.blocks.clone()].concat();
            Ok((fleet.system_path(), tables))
        }
        (Level::User, Some(member)) => Ok((member.path(), vec![Table::Top])),
        (Level::User, None) => bail!("`{}` belongs to a user", key.dotted()),
    }
}

pub fn undeclare(fleet: &Fleet, member: Option<&Member>, keys: &[Key], name: &str) -> Result<()> {
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

pub fn declare(
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

pub fn tool_entry(tool: &Tool) -> Value {
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

pub fn edit_toml(path: &Path, change: impl FnOnce(&mut DocumentMut) -> Result<()>) -> Result<()> {
    let mut doc: DocumentMut = fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?
        .parse()
        .with_context(|| format!("parsing {}", path.display()))?;
    change(&mut doc)?;
    file::write(path, doc.to_string().as_bytes(), FILE_MODE)
}

pub fn array(doc: &mut DocumentMut, key: Key) -> Result<&mut Array> {
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

pub fn find_array<'a>(
    doc: &'a mut DocumentMut,
    table: &Table,
    key: &[&str],
) -> Option<&'a mut Array> {
    let mut item = doc.as_item_mut();
    for part in table.path().into_iter().chain(key.iter().copied()) {
        item = item.get_mut(part)?;
    }
    item.as_array_mut()
}

pub fn push(array: &mut Array, entry: Value) {
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

pub fn remove(array: &mut Array, name: &str) -> bool {
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

pub fn prefix(value: &Value) -> String {
    value
        .decor()
        .prefix()
        .and_then(|p| p.as_str())
        .unwrap_or_default()
        .to_owned()
}

pub fn raw(text: &toml_edit::RawString) -> String {
    text.as_str().unwrap_or_default().to_owned()
}

pub fn names(entry: &Value, name: &str) -> bool {
    entry.as_str() == Some(name)
        || entry
            .as_inline_table()
            .and_then(|t| t.get("name"))
            .and_then(Value::as_str)
            == Some(name)
}
