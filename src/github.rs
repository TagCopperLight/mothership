use crate::{
    cmd::{self, As},
    fleet::checked,
    user::User,
};
use anyhow::{Context, Result};
use serde::Deserialize;

const KEYS_PAGE: &str = "https://github.com/settings/keys";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Use {
    Login,
    Signing,
}

impl Use {
    fn endpoint(self) -> &'static str {
        match self {
            Use::Login => "user/keys",
            Use::Signing => "user/ssh_signing_keys",
        }
    }

    fn shown(self) -> &'static str {
        match self {
            Use::Login => "authentication",
            Use::Signing => "signing",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub title: String,
    pub public: String,
    pub kind: Use,
}

#[derive(Deserialize)]
struct Listed {
    id: u64,
    key: String,
    title: String,
}

pub fn title(host: &str, user: &str) -> String {
    format!("{host}/{user}")
}

fn hosts(url: &str) -> bool {
    url.starts_with("git@github.com:") || url.starts_with("ssh://git@github.com/")
}

fn on_github(admin: &User) -> Result<bool> {
    Ok(hosts(&checked::origin(admin)?))
}

fn listed(admin: &User, kind: Use) -> Result<Vec<Listed>> {
    let page = format!("{}?per_page=100", kind.endpoint());
    let text = cmd::stdout(As::User(admin), "gh", &["api", &page])?;
    serde_json::from_str(&text).context("reading GitHub's list of keys")
}

fn missing<'a>(keys: &[&'a Key], known: &[Listed]) -> Vec<&'a Key> {
    keys.iter()
        .copied()
        .filter(|key| !known.iter().any(|listed| listed.key == key.public))
        .collect()
}

fn retired(host: &str, kind: Use, known: &[Listed]) -> Vec<(u64, String)> {
    let prefix = title(host, "");
    known
        .iter()
        .filter(|listed| match kind {
            Use::Login => listed.title.starts_with(&prefix),
            Use::Signing => listed.title == host,
        })
        .map(|listed| (listed.id, listed.title.clone()))
        .collect()
}

fn gh(admin: &User, args: &[&str]) -> Result<()> {
    cmd::run(
        As::User(admin),
        "gh",
        &[&["api", "--silent"][..], args].concat(),
    )
}

pub fn add(admin: &User, keys: &[Key]) -> Result<()> {
    if keys.is_empty() || !on_github(admin)? {
        return Ok(());
    }
    let mut failed: Vec<(&Key, String)> = Vec::new();
    for kind in [Use::Login, Use::Signing] {
        let wanted: Vec<&Key> = keys.iter().filter(|key| key.kind == kind).collect();
        if wanted.is_empty() {
            continue;
        }
        let known = match listed(admin, kind) {
            Ok(known) => known,
            Err(e) => {
                failed.extend(wanted.iter().map(|key| (*key, format!("{e:#}"))));
                continue;
            }
        };
        for key in missing(&wanted, &known) {
            let title = format!("title={}", key.title);
            let public = format!("key={}", key.public);
            match gh(admin, &["-f", &title, "-f", &public, kind.endpoint()]) {
                Ok(()) => println!("GitHub has {} now.", key.title),
                Err(e) => failed.push((key, format!("{e:#}"))),
            }
        }
    }
    if !failed.is_empty() {
        println!(
            "{}'s gh couldn't add these keys to GitHub, so add them at {KEYS_PAGE}:",
            admin.name
        );
        for (key, why) in failed {
            println!("  {}, {}: {}", key.title, key.kind.shown(), key.public);
            println!("    {}", why.replace('\n', "\n    "));
        }
    }
    Ok(())
}

pub fn remove(admin: &User, host: &str) -> Result<()> {
    if !on_github(admin)? {
        return Ok(());
    }
    for kind in [Use::Login, Use::Signing] {
        let dropped = listed(admin, kind).and_then(|known| {
            for (id, title) in retired(host, kind, &known) {
                gh(
                    admin,
                    &["--method", "DELETE", &format!("{}/{id}", kind.endpoint())],
                )?;
                println!("GitHub drops {title} now.");
            }
            Ok(())
        });
        if let Err(e) = dropped {
            let named = match kind {
                Use::Login => format!("named {}", title(host, "…")),
                Use::Signing => format!("named {host}"),
            };
            println!(
                "{}'s gh couldn't remove the {} keys {named} from GitHub, so remove them at \
                 {KEYS_PAGE}: {e:#}",
                admin.name,
                kind.shown()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/github.rs"]
mod tests;
