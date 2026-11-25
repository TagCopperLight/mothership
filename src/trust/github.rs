use crate::{
    fleet::checkout,
    system::user::User,
    util::cmd::{self, As},
};
use anyhow::{Context, Result};
use serde::Deserialize;

const KEYS_PAGE: &str = "https://github.com/settings/keys";
pub const HOST: &str = "github.com";
pub const META: &str = "https://api.github.com/meta";

#[derive(Deserialize)]
struct Meta {
    ssh_keys: Vec<String>,
}

pub fn known_hosts(meta: &[u8]) -> Result<Vec<String>> {
    let meta: Meta = serde_json::from_slice(meta).context("reading GitHub's host keys")?;
    Ok(meta
        .ssh_keys
        .iter()
        .map(|key| format!("{HOST} {key}"))
        .collect())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Use {
    Login,
    Signing,
    Deploy,
}

const ALL: [Use; 3] = [Use::Login, Use::Signing, Use::Deploy];

impl Use {
    fn endpoint(self, repo: &str) -> String {
        match self {
            Use::Login => "user/keys".to_owned(),
            Use::Signing => "user/ssh_signing_keys".to_owned(),
            Use::Deploy => format!("repos/{repo}/keys"),
        }
    }

    fn shown(self) -> &'static str {
        match self {
            Use::Login => "authentication",
            Use::Signing => "signing",
            Use::Deploy => "fleet deploy",
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

pub fn repo(url: &str) -> Option<String> {
    let path = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))?;
    let path = path.trim_end_matches('/');
    Some(path.strip_suffix(".git").unwrap_or(path).to_owned())
}

fn fleet_repo(admin: &User) -> Result<Option<String>> {
    Ok(repo(&checkout::origin(admin)?))
}

fn listed(admin: &User, kind: Use, repo: &str) -> Result<Vec<Listed>> {
    let page = format!("{}?per_page=100", kind.endpoint(repo));
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
            Use::Login | Use::Deploy => listed.title.starts_with(&prefix),
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
    if keys.is_empty() {
        return Ok(());
    }
    let Some(repo) = fleet_repo(admin)? else {
        return Ok(());
    };
    let mut failed: Vec<(&Key, String)> = Vec::new();
    for kind in ALL {
        let wanted: Vec<&Key> = keys.iter().filter(|key| key.kind == kind).collect();
        if wanted.is_empty() {
            continue;
        }
        let known = match listed(admin, kind, &repo) {
            Ok(known) => known,
            Err(e) => {
                failed.extend(wanted.iter().map(|key| (*key, format!("{e:#}"))));
                continue;
            }
        };
        for key in missing(&wanted, &known) {
            let title = format!("title={}", key.title);
            let public = format!("key={}", key.public);
            let mut args = vec!["-f", &title, "-f", &public];
            if kind == Use::Deploy {
                args.extend(["-F", "read_only=false"]);
            }
            let endpoint = kind.endpoint(&repo);
            args.push(&endpoint);
            match gh(admin, &args) {
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
    let Some(repo) = fleet_repo(admin)? else {
        return Ok(());
    };
    for kind in ALL {
        let dropped = listed(admin, kind, &repo).and_then(|known| {
            for (id, title) in retired(host, kind, &known) {
                gh(
                    admin,
                    &[
                        "--method",
                        "DELETE",
                        &format!("{}/{id}", kind.endpoint(&repo)),
                    ],
                )?;
                println!("GitHub drops {title} now.");
            }
            Ok(())
        });
        if let Err(e) = dropped {
            let named = match kind {
                Use::Login | Use::Deploy => format!("named {}", title(host, "…")),
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
#[path = "../../tests/trust/github.rs"]
mod tests;
