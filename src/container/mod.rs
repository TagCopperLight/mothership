pub mod account;
mod apply;
mod compose;
pub mod confirm;
pub mod deploy;
pub mod docker;
pub mod registry;
pub mod scan;
pub mod state;
pub mod version;

pub use apply::{Action, problems, run};

use crate::{fleet::Fleet, host, secrets::Keys, walk};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub const SERVICES: &str = "services";
pub const SERVICE_TOML: &str = "service.toml";
pub const COMPOSE: &str = "compose.yaml";
pub const SECRETS: &str = "secrets";
const TEMPLATE: &str = ".tmpl";
pub const DATA: &str = "/srv";

#[derive(Deserialize, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct ServiceToml {
    #[serde(default)]
    pub images: BTreeMap<String, Image>,
    #[serde(default)]
    pub snapshot: Vec<PathBuf>,
}

#[derive(Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Image {
    pub image: String,
    pub version: String,
    pub digest: String,
    #[serde(default)]
    pub repo: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub struct Pin {
    pub version: String,
    pub digest: String,
}

impl Image {
    pub fn pin(&self) -> Pin {
        Pin {
            version: self.version.clone(),
            digest: self.digest.clone(),
        }
    }

    pub fn reference(&self, pin: &Pin) -> String {
        format!("{}:{}@{}", self.image, pin.version, pin.digest)
    }

    fn check(&self) -> Result<()> {
        let path = self
            .image
            .split_once('/')
            .map_or(&self.image[..], |(_, p)| p);
        let plain = !self.image.is_empty()
            && !path.contains(':')
            && !self.image.contains('@')
            && self
                .image
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"./-_:".contains(&b));
        if !plain {
            bail!("{} isn't an image name without a tag", self.image);
        }
        version::Version::parse(&self.version)
            .with_context(|| format!("{}: the fleet runs only images with versions", self.image))?;
        check_digest(&self.digest)?;
        if let Some(repo) = &self.repo
            && repo.split('/').count() != 2
        {
            bail!("{repo} isn't a GitHub repository like owner/name");
        }
        Ok(())
    }
}

pub fn check_digest(digest: &str) -> Result<()> {
    let hex = digest.strip_prefix("sha256:").unwrap_or("");
    if hex.len() != 64 || !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        bail!("{digest} isn't a digest like sha256:…");
    }
    Ok(())
}

pub fn parse_toml(text: &str) -> Result<ServiceToml> {
    let parsed: ServiceToml = toml::from_str(text)?;
    if parsed.images.is_empty() {
        bail!("it pins no image");
    }
    for (name, image) in &parsed.images {
        image.check().with_context(|| format!("images.{name}"))?;
    }
    for part in &parsed.snapshot {
        let plain = part
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)));
        if !plain || part.as_os_str().is_empty() {
            bail!(
                "snapshot names {}, which isn't a path inside the service's data folder",
                part.display()
            );
        }
    }
    Ok(parsed)
}

pub fn parse(text: &str) -> Result<BTreeMap<String, Image>> {
    parse_toml(text).map(|parsed| parsed.images)
}

pub const LOGINS: &str = "docker-config.json.age";

pub fn sealed(type_dir: &Path) -> Result<Vec<PathBuf>> {
    let dir = type_dir.join(SERVICES);
    Ok(walk::files(&dir)?
        .into_iter()
        .filter(|rel| rel.extension().is_some_and(|e| e == "age"))
        .map(|rel| dir.join(rel))
        .collect())
}

pub fn logins_path(fleet: &Fleet) -> PathBuf {
    fleet.type_dir.join(SERVICES).join(LOGINS)
}

pub fn logins(fleet: &Fleet) -> Result<Option<Vec<u8>>> {
    let path = logins_path(fleet);
    match path.exists() {
        true => fleet.root_keys().decrypt(&path).map(Some),
        false => Ok(None),
    }
}

pub fn is_service_toml(path: &Path) -> bool {
    service_of(path).is_some()
}

pub fn service_of(path: &Path) -> Option<String> {
    let parts: Vec<&str> = path.iter().map(|p| p.to_str()).collect::<Option<_>>()?;
    match parts[..] {
        ["types", type_name, SERVICES, name, SERVICE_TOML] => Some(format!("{type_name}/{name}")),
        _ => None,
    }
}

pub fn versions_only(before: &str, after: &str) -> Result<()> {
    let (old, new) = (parse_toml(before)?, parse_toml(after)?);
    if old.snapshot != new.snapshot {
        bail!("it changes what the snapshot covers");
    }
    let (old, new) = (old.images, new.images);
    if old.keys().ne(new.keys()) {
        bail!("it adds or removes an image");
    }
    for (name, old) in &old {
        let new = &new[name];
        let repinned = Image {
            version: old.version.clone(),
            digest: old.digest.clone(),
            ..new.clone()
        };
        if *old != repinned {
            bail!("it changes images.{name} beyond its version and digest");
        }
        if old.pin() != new.pin() {
            version::check_update(&old.version, &new.version)
                .with_context(|| format!("images.{name}"))?;
        }
    }
    Ok(())
}

pub fn changed_pins(before: Option<&str>, after: Option<&str>) -> Vec<(String, Pin)> {
    let read = |text: Option<&str>| text.and_then(|t| parse(t).ok()).unwrap_or_default();
    let (old, new) = (read(before), read(after));
    new.into_iter()
        .filter(|(name, image)| old.get(name).map(Image::pin) != Some(image.pin()))
        .map(|(name, image)| (name, image.pin()))
        .collect()
}

#[derive(Clone)]
pub struct Service {
    pub name: String,
    pub dir: PathBuf,
    pub images: BTreeMap<String, Image>,
    pub snapshot: Vec<PathBuf>,
}

pub fn load(type_dir: &Path) -> Result<Vec<Service>> {
    let mut services = Vec::new();
    for dir in walk::children(&type_dir.join(SERVICES))? {
        if !dir.is_dir() {
            continue;
        }
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .with_context(|| format!("{} isn't a service name", dir.display()))?
            .to_owned();
        host::check(&name).with_context(|| format!("the service {}", dir.display()))?;
        let path = dir.join(SERVICE_TOML);
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let ServiceToml { images, snapshot } =
            parse_toml(&text).with_context(|| format!("parsing {}", path.display()))?;
        if !dir.join(COMPOSE).is_file() {
            bail!("{} has no {COMPOSE}", dir.display());
        }
        services.push(Service {
            name,
            dir,
            images,
            snapshot,
        });
    }
    Ok(services)
}

#[derive(Debug)]
pub struct Rendered {
    pub files: BTreeMap<PathBuf, Vec<u8>>,
    pub secrets: BTreeMap<String, Vec<u8>>,
}

impl Service {
    pub fn pins(&self) -> BTreeMap<String, Pin> {
        self.images
            .iter()
            .map(|(name, image)| (name.clone(), image.pin()))
            .collect()
    }

    pub fn secret_files(&self) -> Result<Vec<PathBuf>> {
        Ok(walk::files(&self.dir.join(SECRETS))?
            .into_iter()
            .map(|file| self.dir.join(SECRETS).join(file))
            .collect())
    }

    pub fn render(&self, keys: &Keys) -> Result<Rendered> {
        let mut secrets = BTreeMap::new();
        for path in self.secret_files()? {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(crate::secrets::AGE))
                .with_context(|| format!("{} isn't a secret.age file", path.display()))?;
            host::check(name).with_context(|| format!("the secret {}", path.display()))?;
            secrets.insert(name.to_owned(), keys.decrypt(&path)?);
        }
        let mut files = BTreeMap::new();
        for rel in walk::files(&self.dir)? {
            if rel == Path::new(SERVICE_TOML) || rel.starts_with(SECRETS) {
                continue;
            }
            let path = self.dir.join(&rel);
            if fs::symlink_metadata(&path)?.is_symlink() {
                bail!("{} is a link: a service holds plain files", path.display());
            }
            let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            match rel.to_str().and_then(|r| r.strip_suffix(TEMPLATE)) {
                Some(target) => {
                    let text = String::from_utf8(bytes)
                        .with_context(|| format!("{} isn't text", path.display()))?;
                    let filled = fill(&text, &secrets)
                        .with_context(|| format!("filling {}", path.display()))?;
                    files.insert(PathBuf::from(target), filled.into_bytes());
                }
                None => {
                    files.insert(rel, bytes);
                }
            }
        }
        for rel in files.keys() {
            if rel.starts_with(SECRETS) {
                bail!("{}: secrets/ holds only encrypted secrets", rel.display());
            }
        }
        Ok(Rendered { files, secrets })
    }
}

fn fill(text: &str, secrets: &BTreeMap<String, Vec<u8>>) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let end = rest[start..].find("}}").context("a {{ without its }}")? + start;
        let inside = rest[start + 2..end].trim();
        let name = inside
            .strip_prefix("secret")
            .map(str::trim)
            .and_then(|n| n.strip_prefix('"'))
            .and_then(|n| n.strip_suffix('"'))
            .with_context(|| format!("{{{{ {inside} }}}} isn't {{{{ secret \"name\" }}}}"))?;
        let value = secrets
            .get(name)
            .with_context(|| format!("the service has no secret {name}"))?;
        out.push_str(
            std::str::from_utf8(value)
                .with_context(|| format!("the secret {name} isn't text"))?
                .trim_end_matches('\n'),
        );
        rest = &rest[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
#[path = "../../tests/container.rs"]
mod tests;
