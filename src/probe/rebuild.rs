use super::{plan::Watched, state::State};
use crate::{
    container::registry::Client,
    http::{Http, Request},
};
use anyhow::{Context, Result, bail};

const API: &str = "https://api.github.com";
const EVENT: &str = "rebuild";

pub struct GitHub<'a> {
    pub http: &'a dyn Http,
    pub token: String,
}

impl GitHub<'_> {
    fn headers(&self, accept: &str) -> Vec<String> {
        vec![
            format!("Accept: {accept}"),
            format!("Authorization: Bearer {}", self.token),
            "X-GitHub-Api-Version: 2022-11-28".to_owned(),
            "User-Agent: mothership-probe".to_owned(),
        ]
    }

    fn dockerfile(&self, repo: &str, version: &str) -> Result<String> {
        let number = version.strip_prefix('v').unwrap_or(version);
        for tag in [format!("v{number}"), number.to_owned()] {
            let url = format!("{API}/repos/{repo}/contents/Dockerfile?ref={tag}");
            let response = self.http.send(&Request {
                method: "GET",
                url: &url,
                headers: self.headers("application/vnd.github.raw+json"),
                body: None,
            })?;
            if response.status == 404 {
                continue;
            }
            let response = response.ok(&format!("{repo}'s Dockerfile at {tag}"))?;
            return String::from_utf8(response.body).context("a Dockerfile that isn't text");
        }
        bail!("{repo} has no Dockerfile at a tag v{number} or {number}")
    }

    fn dispatch(&self, repo: &str, payload: serde_json::Value) -> Result<()> {
        let url = format!("{API}/repos/{repo}/dispatches");
        let body = serde_json::json!({ "event_type": EVENT, "client_payload": payload });
        self.http
            .send(&Request {
                method: "POST",
                url: &url,
                headers: self.headers("application/vnd.github+json"),
                body: Some(body.to_string().as_bytes()),
            })?
            .ok(&format!("{repo}'s rebuild event"))
            .map(drop)
    }
}

pub fn bases(dockerfile: &str) -> Vec<(String, String)> {
    let mut stages = Vec::new();
    let mut bases = Vec::new();
    for line in dockerfile.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some((first, rest)) = words.split_first() else {
            continue;
        };
        if !first.eq_ignore_ascii_case("FROM") {
            continue;
        }
        let rest: Vec<&str> = rest
            .iter()
            .copied()
            .filter(|w| !w.starts_with("--"))
            .collect();
        let Some(reference) = rest.first() else {
            continue;
        };
        let known = stages.contains(&reference.to_lowercase());
        if let [_, as_word, name] = rest[..]
            && as_word.eq_ignore_ascii_case("AS")
        {
            stages.push(name.to_lowercase());
        }
        if known || *reference == "scratch" || reference.contains('$') || reference.contains('@') {
            continue;
        }
        let tail = reference.rsplit('/').next().unwrap_or(reference);
        let (image, tag) = match tail.split_once(':') {
            Some((_, tag)) => (&reference[..reference.len() - tag.len() - 1], tag),
            None => (*reference, "latest"),
        };
        bases.push((image.to_owned(), tag.to_owned()));
    }
    bases.sort();
    bases.dedup();
    bases
}

pub struct Asked {
    pub asked: Vec<String>,
    pub failures: Vec<String>,
}

struct Rebuild<'a> {
    repo: &'a str,
    key: String,
    shown: String,
    payload: serde_json::Value,
}

pub fn ask(
    github: &GitHub,
    registry: &Client,
    watched: &[Watched],
    fleet_repo: Option<&str>,
    state: &mut State,
) -> Asked {
    let mut done = Asked {
        asked: Vec::new(),
        failures: Vec::new(),
    };
    for item in watched {
        let version = &item.image.version;
        let (rebuild, dockerfile) = match (&item.dockerfile, &item.image.repo, fleet_repo) {
            (Some(dockerfile), _, Some(fleet)) => (
                Rebuild {
                    repo: fleet,
                    key: format!(
                        "{fleet}:{}@{}",
                        item.service,
                        crate::hash::sha256(dockerfile.as_bytes())
                    ),
                    shown: format!("the fleet's recipe {}", item.service),
                    payload: serde_json::json!({ "service": item.service }),
                },
                Ok(dockerfile.clone()),
            ),
            (None, Some(repo), _) => (
                Rebuild {
                    repo,
                    key: format!("{repo}@{version}"),
                    shown: format!("{repo} {version}"),
                    payload: serde_json::json!({ "version": version }),
                },
                github.dockerfile(repo, version),
            ),
            _ => continue,
        };
        let result = dockerfile.and_then(|text| ask_one(github, registry, &text, &rebuild, state));
        match result {
            Ok(true) => done.asked.push(rebuild.shown),
            Ok(false) => {}
            Err(e) => done.failures.push(format!("{}: {e:#}", rebuild.shown)),
        }
    }
    done
}

fn ask_one(
    github: &GitHub,
    registry: &Client,
    dockerfile: &str,
    rebuild: &Rebuild,
    state: &mut State,
) -> Result<bool> {
    let mut now = std::collections::BTreeMap::new();
    for (image, tag) in bases(dockerfile) {
        now.insert(format!("{image}:{tag}"), registry.digest(&image, &tag)?);
    }
    let changed = state
        .bases
        .get(&rebuild.key)
        .is_some_and(|before| *before != now);
    if changed {
        github.dispatch(rebuild.repo, rebuild.payload.clone())?;
    }
    state.bases.insert(rebuild.key.clone(), now);
    Ok(changed)
}

#[cfg(test)]
#[path = "../../tests/rebuild.rs"]
mod tests;
