use crate::http::{Http, Request, Response};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Mutex,
};

const ACCEPT: &str = "Accept: application/vnd.oci.image.index.v1+json, \
    application/vnd.docker.distribution.manifest.list.v2+json, \
    application/vnd.docker.distribution.manifest.v2+json, \
    application/vnd.oci.image.manifest.v1+json";
const DOCKER_HUB: &str = "docker.io";
const DOCKER_HUB_API: &str = "registry-1.docker.io";
const TAGS_PAGE: usize = 1000;

#[derive(Deserialize, Default)]
struct Config {
    #[serde(default)]
    auths: BTreeMap<String, Login>,
}

#[derive(Deserialize)]
struct Login {
    auth: String,
}

pub struct Client {
    http: Box<dyn Http>,
    logins: BTreeMap<String, String>,
    tokens: Mutex<HashMap<String, String>>,
}

#[derive(Debug, PartialEq)]
pub struct Name {
    pub host: String,
    pub path: String,
}

pub fn name(image: &str) -> Name {
    let (first, rest) = image.split_once('/').unwrap_or(("", image));
    let is_host = first.contains(['.', ':']) || first == "localhost";
    let (host, path) = match is_host {
        true => (first.to_owned(), rest.to_owned()),
        false => (DOCKER_HUB.to_owned(), image.to_owned()),
    };
    let path = match (host.as_str(), path.contains('/')) {
        (DOCKER_HUB, false) => format!("library/{path}"),
        _ => path,
    };
    Name { host, path }
}

impl Name {
    fn api(&self) -> &str {
        match self.host.as_str() {
            DOCKER_HUB => DOCKER_HUB_API,
            host => host,
        }
    }
}

#[derive(Deserialize)]
struct Tags {
    #[serde(default)]
    tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct Token {
    token: Option<String>,
    access_token: Option<String>,
}

impl Client {
    pub fn new(http: Box<dyn Http>, docker_config: Option<&[u8]>) -> Result<Client> {
        let config: Config = match docker_config {
            Some(bytes) => serde_json::from_slice(bytes).context("reading the Docker logins")?,
            None => Config::default(),
        };
        let logins = config
            .auths
            .into_iter()
            .map(|(host, login)| {
                let host = host.trim_start_matches("https://").trim_end_matches('/');
                let host = match host {
                    "index.docker.io/v1" | "index.docker.io" => DOCKER_HUB,
                    host => host,
                };
                (host.to_owned(), login.auth)
            })
            .collect();
        Ok(Client {
            http,
            logins,
            tokens: Mutex::new(HashMap::new()),
        })
    }

    pub fn digest(&self, image: &str, tag: &str) -> Result<String> {
        let name = name(image);
        let url = format!("https://{}/v2/{}/manifests/{tag}", name.api(), name.path);
        let response = self.get(&name, "HEAD", &url, &[ACCEPT.to_owned()])?;
        let response = response.ok(&format!("{image}:{tag}"))?;
        let digest = response
            .header("docker-content-digest")
            .with_context(|| format!("the registry gave no digest for {image}:{tag}"))?;
        super::check_digest(digest)?;
        Ok(digest.to_owned())
    }

    pub fn tags(&self, image: &str) -> Result<Vec<String>> {
        let name = name(image);
        let mut url = format!(
            "https://{}/v2/{}/tags/list?n={TAGS_PAGE}",
            name.api(),
            name.path
        );
        let mut all = Vec::new();
        loop {
            let response = self.get(&name, "GET", &url, &[])?.ok(image)?;
            let page: Tags = serde_json::from_slice(&response.body)
                .with_context(|| format!("reading {image}'s tags"))?;
            all.extend(page.tags.unwrap_or_default());
            match response.header("link").and_then(next_page) {
                Some(next) if next.starts_with('/') => {
                    url = format!("https://{}{next}", name.api())
                }
                Some(next) => bail!("{image}'s registry pages its tags to {next}"),
                None => return Ok(all),
            }
        }
    }

    fn get(&self, name: &Name, method: &str, url: &str, extra: &[String]) -> Result<Response> {
        let send = |token: Option<&str>| {
            let mut headers = extra.to_vec();
            if let Some(token) = token {
                headers.push(format!("Authorization: Bearer {token}"));
            }
            self.http.send(&Request {
                method,
                url,
                headers,
                body: None,
            })
        };
        let key = format!("{}/{}", name.host, name.path);
        let cached = self
            .tokens
            .lock()
            .expect("no panic holds it")
            .get(&key)
            .cloned();
        let response = send(cached.as_deref())?;
        if response.status != 401 {
            return Ok(response);
        }
        let challenge = response
            .header("www-authenticate")
            .context("the registry refused without saying how to log in")?
            .to_owned();
        let token = self.token(name, &challenge)?;
        self.tokens
            .lock()
            .expect("no panic holds it")
            .insert(key, token.clone());
        send(Some(&token))
    }

    fn token(&self, name: &Name, challenge: &str) -> Result<String> {
        let params = challenge
            .strip_prefix("Bearer ")
            .with_context(|| format!("the registry asks for {challenge}"))?;
        let mut realm = None;
        let mut query = Vec::new();
        for (key, value) in parse_params(params) {
            match key.as_str() {
                "realm" => realm = Some(value),
                _ => query.push(format!("{key}={value}")),
            }
        }
        let realm = realm.context("the registry's login names no realm")?;
        if !realm.starts_with("https://") {
            bail!("the registry logs in through {realm}, which isn't https");
        }
        let url = format!("{realm}?{}", query.join("&"));
        let headers = match self.logins.get(&name.host) {
            Some(auth) => vec![format!("Authorization: Basic {auth}")],
            None => Vec::new(),
        };
        let response = self
            .http
            .send(&Request {
                method: "GET",
                url: &url,
                headers,
                body: None,
            })?
            .ok(&format!("{}'s login", name.host))?;
        let token: Token =
            serde_json::from_slice(&response.body).context("reading the registry's token")?;
        token
            .token
            .or(token.access_token)
            .context("the registry gave no token")
    }
}

fn parse_params(text: &str) -> Vec<(String, String)> {
    let mut params = Vec::new();
    let mut rest = text.trim();
    while let Some((key, after)) = rest.split_once('=') {
        let key = key.trim().trim_start_matches(',').trim().to_owned();
        let (value, next) = match after.strip_prefix('"') {
            Some(quoted) => quoted.split_once('"').unwrap_or((quoted, "")),
            None => after.split_once(',').unwrap_or((after, "")),
        };
        params.push((key, value.to_owned()));
        rest = next.trim_start_matches(',').trim();
    }
    params
}

fn next_page(link: &str) -> Option<&str> {
    let (target, rel) = link.split_once(';')?;
    rel.contains("rel=\"next\"")
        .then(|| target.trim().trim_start_matches('<').trim_end_matches('>'))
}

#[cfg(test)]
#[path = "../../tests/registry.rs"]
mod tests;
