use super::{
    COMPOSE, Rendered, SECRETS, Service, compose,
    confirm::Unconfirmed,
    docker::{Engine, Health, Project},
    state::{Deployed, Pins, Problem},
};
use crate::{file, hash};
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub const PROJECTS: &str = "/var/lib/mothership/services";
pub const OVERRIDE: &str = "mothership.json";

pub struct Places {
    pub projects: PathBuf,
    pub data: PathBuf,
    pub owner: (u32, u32),
}

impl Places {
    pub fn here() -> Result<Places> {
        let owner = super::account::user()?.map_or((0, 0), |user| (user.uid, user.gid));
        Ok(Places {
            projects: PathBuf::from(PROJECTS),
            data: PathBuf::from(super::DATA),
            owner,
        })
    }

    pub fn project(&self, name: &str) -> PathBuf {
        self.projects.join(name)
    }

    pub fn data(&self, name: &str) -> PathBuf {
        self.data.join(name)
    }
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Started,
    Updated,
    Unhealthy(String),
    RolledBack(String),
    Blocked(String),
    Refused(String),
}

pub fn fingerprint(service: &Service, rendered: &Rendered, logins: Option<&[u8]>) -> String {
    let mut fields = hash::Fields::new();
    fields.add(logins.unwrap_or_default());
    for (image, pin) in service.pins() {
        for part in [image, pin.version, pin.digest] {
            fields.add(part.as_bytes());
        }
    }
    for (rel, bytes) in &rendered.files {
        fields.add(rel.as_os_str().as_encoded_bytes());
        fields.add(bytes);
    }
    for (name, bytes) in &rendered.secrets {
        fields.add(name.as_bytes());
        fields.add(bytes);
    }
    fields.finish()
}

pub fn edited(places: &Places, name: &str, entry: &Deployed) -> Result<Vec<PathBuf>> {
    let dir = places.project(name);
    let mut edited = Vec::new();
    for (rel, written) in &entry.files {
        let path = dir.join(rel);
        let now = file::if_found(fs::read(&path))
            .with_context(|| format!("reading {}", path.display()))?;
        if let Some(bytes) = now
            && hash::md5(&bytes) != *written
        {
            edited.push(rel.clone());
        }
    }
    Ok(edited)
}

fn write_files(
    places: &Places,
    service: &Service,
    rendered: &Rendered,
    before: &Deployed,
) -> Result<BTreeMap<PathBuf, String>> {
    let dir = places.project(&service.name);
    private_dir(&dir, places.owner.1)?;
    let edited = edited(places, &service.name, before)?;
    let mut wanted: BTreeMap<PathBuf, &[u8]> = rendered
        .files
        .iter()
        .map(|(rel, bytes)| (rel.clone(), &bytes[..]))
        .collect();
    if wanted.contains_key(Path::new(OVERRIDE)) {
        bail!(
            "{}/{OVERRIDE} is mothership's own file",
            service.dir.display()
        );
    }
    for (name, bytes) in &rendered.secrets {
        wanted.insert(Path::new(SECRETS).join(name), bytes);
    }
    let mut written = BTreeMap::new();
    for (rel, bytes) in &wanted {
        if edited.contains(rel) {
            written.insert(rel.clone(), before.files[rel].clone());
            continue;
        }
        let path = dir.join(rel);
        file::write(&path, bytes, 0o644).with_context(|| format!("writing {}", path.display()))?;
        written.insert(rel.clone(), hash::md5(bytes));
    }
    for rel in before.files.keys() {
        if !wanted.contains_key(rel) && !edited.contains(rel) {
            file::remove_if_found(&dir.join(rel))?;
        }
    }
    Ok(written)
}

fn private_dir(dir: &Path, gid: u32) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    std::os::unix::fs::chown(dir, None, Some(gid))
        .with_context(|| format!("giving {} to the services account", dir.display()))?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o750))?;
    Ok(())
}

fn write_override(dir: &Path, service: &Service, pins: &Pins) -> Result<BTreeMap<String, String>> {
    let mut images = BTreeMap::new();
    for (name, pin) in pins {
        let image = service
            .images
            .get(name)
            .with_context(|| format!("{} no longer pins images.{name}", service.name))?;
        images.insert(name.clone(), image.reference(pin));
    }
    let services: serde_json::Map<String, serde_json::Value> = images
        .iter()
        .map(|(name, image)| (name.clone(), serde_json::json!({ "image": image })))
        .collect();
    let text = serde_json::to_string_pretty(&serde_json::json!({ "services": services }))?;
    file::write(&dir.join(OVERRIDE), text.as_bytes(), 0o644)?;
    Ok(images)
}

fn data_dir(places: &Places, name: &str) -> Result<PathBuf> {
    let data = places.data(name);
    if fs::symlink_metadata(&data).is_err() {
        fs::create_dir_all(&places.data)?;
        fs::create_dir(&data).with_context(|| format!("creating {}", data.display()))?;
        std::os::unix::fs::chown(&data, Some(places.owner.0), Some(places.owner.1))
            .with_context(|| format!("giving {} to the services account", data.display()))?;
        fs::set_permissions(&data, fs::Permissions::from_mode(0o700))?;
    } else if fs::symlink_metadata(&data)?.is_symlink() {
        bail!("{} is a link", data.display());
    }
    Ok(data)
}

fn env(rendered: &Rendered) -> Vec<(String, String)> {
    rendered
        .secrets
        .iter()
        .filter_map(|(name, bytes)| {
            let value = std::str::from_utf8(bytes).ok()?.trim_end_matches('\n');
            Some((
                format!("SECRET_{}", name.to_uppercase().replace('-', "_")),
                value.to_owned(),
            ))
        })
        .collect()
}

pub struct Wanted<'a> {
    pub service: &'a Service,
    pub key: String,
    pub rendered: &'a Rendered,
    pub fleet: String,
}

pub fn deploy(
    engine: &dyn Engine,
    places: &Places,
    wanted: &Wanted,
    before: Option<&Deployed>,
    unconfirmed: &mut Unconfirmed,
) -> Result<(Deployed, Outcome)> {
    let service = wanted.service;
    let name = &service.name;
    let pins = service.pins();
    let mut entry = before.cloned().unwrap_or_default();
    let blocked = describe(&entry.failed_in(&pins));
    if !blocked.is_empty() {
        entry.fleet = wanted.fleet.clone();
        entry.problem = Some(Problem {
            fleet: wanted.fleet.clone(),
            why: format!("{blocked} failed its health check here before"),
        });
        return Ok((entry, Outcome::Blocked(blocked)));
    }
    for (image_name, image) in &service.images {
        if !unconfirmed.waits(&wanted.key, image_name, &image.pin()) {
            continue;
        }
        if let Some(why) = engine.confirm(image, &image.pin())? {
            entry.fleet = wanted.fleet.clone();
            entry.problem = Some(Problem {
                fleet: wanted.fleet.clone(),
                why: why.clone(),
            });
            return Ok((entry, Outcome::Refused(why)));
        }
        unconfirmed.confirm(&wanted.key, image_name);
    }

    entry.files = write_files(places, service, wanted.rendered, &entry)?;
    let dir = places.project(name);
    let project = Project {
        name,
        dir: dir.clone(),
        env: env(wanted.rendered),
    };
    let images = write_override(&dir, service, &pins)?;
    let data = data_dir(places, name)?;
    compose::check(&engine.config(&project)?, &dir, &data, &images)
        .with_context(|| format!("checking {}/{COMPOSE}", service.dir.display()))?;
    engine.pull(&project)?;
    let update = !entry.running.is_empty() && entry.running != pins && engine.runs(name)?;
    if update {
        engine.stop(name)?;
        engine.snapshot(name, &data)?;
    }
    engine.up(&project)?;
    let health = engine.health(name)?;
    entry.fleet = wanted.fleet.clone();
    let outcome = match health {
        Health::Unhealthy(why) if update => {
            let back = entry
                .healthy
                .clone()
                .unwrap_or_else(|| entry.running.clone());
            engine.stop(name)?;
            engine.restore(name, &data)?;
            write_override(&dir, service, &back)?;
            engine.up(&project)?;
            for (image, pin) in &pins {
                if back.get(image) != Some(pin) {
                    entry.failed.insert((image.clone(), pin.clone()));
                }
            }
            entry.running = back;
            Outcome::RolledBack(why)
        }
        Health::Unhealthy(why) => {
            entry.running = pins;
            Outcome::Unhealthy(why)
        }
        Health::Healthy | Health::Unchecked => {
            if health == Health::Healthy {
                entry.healthy = Some(pins.clone());
            }
            let first = entry.running.is_empty();
            entry.running = pins;
            match first {
                true => Outcome::Started,
                false => Outcome::Updated,
            }
        }
    };
    entry.problem = match &outcome {
        Outcome::RolledBack(why) => Some(format!(
            "{} failed its health check ({why}), so it runs {} again",
            describe(&entry.failed_in(&wanted.service.pins())),
            describe(&entry.running.iter().collect::<Vec<_>>())
        )),
        Outcome::Unhealthy(why) => Some(format!("it isn't healthy: {why}")),
        _ => None,
    }
    .map(|why| Problem {
        fleet: wanted.fleet.clone(),
        why,
    });
    Ok((entry, outcome))
}

pub fn describe(pins: &[(&String, &super::Pin)]) -> String {
    pins.iter()
        .map(|(image, pin)| format!("images.{image} {}", pin.version))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn remove(engine: &dyn Engine, places: &Places, name: &str, entry: &Deployed) -> Result<()> {
    engine.down(name)?;
    let dir = places.project(name);
    let edited = edited(places, name, entry)?;
    for rel in entry.files.keys().filter(|rel| !edited.contains(rel)) {
        file::remove_if_found(&dir.join(rel))?;
    }
    file::remove_if_found(&dir.join(OVERRIDE))?;
    let mut dirs: Vec<PathBuf> = entry
        .files
        .keys()
        .flat_map(|rel| rel.ancestors().skip(1))
        .map(|ancestor| dir.join(ancestor))
        .collect();
    dirs.push(dir.clone());
    dirs.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    dirs.dedup();
    for path in dirs {
        if fs::read_dir(&path).is_ok_and(|mut entries| entries.next().is_none()) {
            fs::remove_dir(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/deploy.rs"]
mod tests;
