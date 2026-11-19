use super::{Change, Item};
use crate::kind::Kind;
use crate::{
    cmd::{self, As},
    edit::Add,
    fleet::Member,
    pref,
    preview::Preview,
    style,
    user::User,
};
use anyhow::Result;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub fn scan(member: &Member, managed: &[PathBuf]) -> Result<Vec<Item>> {
    let mut items = dconf(member.user())?;
    for user_js in managed.iter().filter(|path| path.ends_with("user.js")) {
        items.extend(prefs(member.user(), user_js)?);
    }
    Ok(items)
}

fn dconf(user: &User) -> Result<Vec<Item>> {
    if !user.home.join(".config/dconf/user").exists() {
        return Ok(Vec::new());
    }
    let dump = cmd::stdout(As::User(user), "dconf", &["dump", "/"])?;
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
        .map(|(section, keys)| {
            let keys = style::count(keys, "key");
            Item::new(Kind::Dconf, Change::Added, &section, "set")
                .noting(keys)
                .showing(Preview::Dconf {
                    section,
                    user: user.clone(),
                })
        })
        .collect())
}

fn prefs(user: &User, user_js: &Path) -> Result<Vec<Item>> {
    let read = |path: &Path| -> Result<Option<String>> {
        Ok(user
            .read(path)?
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
    };
    let Some(wanted) = read(user_js)? else {
        return Ok(Vec::new());
    };
    let Some(actual) = read(&user_js.with_file_name("prefs.js"))? else {
        return Ok(Vec::new());
    };
    let (wanted, actual) = (pref::parse(&wanted), pref::parse(&actual));
    Ok(actual
        .iter()
        .filter_map(|(key, value)| {
            let (change, detail, note) = match wanted.get(key) {
                None => (Change::Added, "set in the app", format!("= {value}")),
                Some(w) if w != value => (
                    Change::Edited,
                    "changed in the app",
                    format!("{w} → {value}, user.js resets it"),
                ),
                Some(_) => return None,
            };
            Some(
                Item::new(Kind::Pref, change, key, detail)
                    .noting(note)
                    .with(Add::Pref {
                        user_js: user_js.to_path_buf(),
                        key: key.clone(),
                        value: value.clone(),
                    }),
            )
        })
        .collect())
}

#[cfg(test)]
#[path = "../../tests/settings.rs"]
mod tests;
