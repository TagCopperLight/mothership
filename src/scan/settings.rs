use super::{Item, Kind};
use crate::cmd;
use anyhow::Result;
use std::{collections::BTreeMap, fs, path::PathBuf};

/// Settings apps keep as data rather than as config files.
pub fn scan(managed: &[PathBuf]) -> Result<Vec<Item>> {
    let mut items = dconf()?;
    for user_js in managed.iter().filter(|path| path.ends_with("user.js")) {
        items.extend(prefs(user_js)?);
    }
    Ok(items)
}

/// The fleet holds no dconf settings yet, so every section set on the machine counts.
fn dconf() -> Result<Vec<Item>> {
    let dump = cmd::stdout("dconf", &["dump", "/"])?;
    let mut sections: BTreeMap<String, usize> = BTreeMap::new();
    let mut current = None;
    for line in dump.lines() {
        if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = Some(format!("/{section}"));
        } else if line.contains('=')
            && let Some(section) = &current
        {
            *sections.entry(section.clone()).or_default() += 1;
        }
    }
    Ok(sections
        .into_iter()
        .map(|(section, keys)| Item::new(Kind::Dconf, section, format!("{keys} keys set")))
        .collect())
}

/// Firefox-family browsers save UI changes to prefs.js, next to the user.js the fleet holds.
/// user.js wins at every start, so a pref changed in the app that user.js also sets is lost.
fn prefs(user_js: &PathBuf) -> Result<Vec<Item>> {
    let wanted = parse(&fs::read_to_string(user_js)?);
    let actual = parse(&fs::read_to_string(user_js.with_file_name("prefs.js"))?);
    Ok(actual
        .iter()
        .filter_map(|(key, value)| match wanted.get(key) {
            None => Some(Item::new(Kind::Pref, key, "set in the app, not in user.js")),
            Some(w) if w != value => Some(Item::new(
                Kind::Pref,
                key,
                "changed in the app, user.js resets it",
            )),
            Some(_) => None,
        })
        .collect())
}

fn parse(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("user_pref(\"")?.strip_suffix(");"))
        .filter_map(|rest| rest.split_once("\", "))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}
