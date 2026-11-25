use super::{
    COMPOSE, Published, Rendered, SECRETS, Service, compose,
    confirm::Unconfirmed,
    docker::{Engine, Health, Project},
    link,
    port::{self, Bound},
    state::{Deployed, Pins, Problem},
};
use crate::{util::file, util::hash, util::walk};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub const PROJECTS: &str = "/var/lib/mothership/services";
pub const OVERRIDE: &str = "mothership.json";
pub const OWN: &str = "x-mothership";

pub type Lookup = fn(&str) -> Result<Vec<std::net::IpAddr>>;

pub struct Places {
    pub projects: PathBuf,
    pub data: PathBuf,
    pub owner: (u32, u32),
    pub lookup: Lookup,
}

impl Places {
    pub fn here() -> Result<Places> {
        let owner = super::account::user()?.map_or((0, 0), |user| (user.uid, user.gid));
        Ok(Places {
            projects: PathBuf::from(PROJECTS),
            data: PathBuf::from(super::DATA),
            owner,
            lookup: port::addresses,
        })
    }

    pub fn project(&self, name: &str) -> PathBuf {
        self.projects.join(name)
    }

    pub fn previous(&self, name: &str) -> PathBuf {
        self.projects.join(format!(".{name}.previous"))
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

pub fn fingerprint(service: &Service, rendered: &Rendered) -> String {
    let mut fields = hash::Fields::new();
    for (name, image) in &service.images {
        for part in [name, &image.version, &image.digest] {
            fields.add(part.as_bytes());
        }
        for published in &image.ports {
            fields.add(published.to_string().as_bytes());
        }
    }
    for link in &service.links {
        fields.add(link.network().as_bytes());
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
    wanted: &Wanted,
    before: &Deployed,
) -> Result<BTreeMap<PathBuf, String>> {
    let (service, rendered) = (wanted.service, wanted.rendered);
    let dir = places.project(&service.name);
    private_dir(&dir, places.owner.1)?;
    let edited = edited(places, &service.name, before)?;
    let mut files: BTreeMap<PathBuf, &[u8]> = rendered
        .files
        .iter()
        .map(|(rel, bytes)| (rel.clone(), &bytes[..]))
        .collect();
    if files.contains_key(Path::new(OVERRIDE)) {
        bail!(
            "{}/{OVERRIDE} is mothership's own file",
            service.dir.display()
        );
    }
    for (name, bytes) in &rendered.secrets {
        files.insert(Path::new(SECRETS).join(name), bytes);
    }
    let mut written = BTreeMap::new();
    for (rel, bytes) in &files {
        if edited.contains(rel) && !wanted.overwrite.contains(rel) {
            written.insert(rel.clone(), before.files[rel].clone());
            continue;
        }
        let path = dir.join(rel);
        file::write(&path, bytes, 0o644).with_context(|| format!("writing {}", path.display()))?;
        written.insert(rel.clone(), hash::md5(bytes));
    }
    for rel in before.files.keys() {
        if !files.contains_key(rel) && !edited.contains(rel) {
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

#[derive(Clone, Copy)]
enum Stage<'a> {
    Checked,
    Published(&'a Value),
}

fn write_override(
    places: &Places,
    service: &Service,
    pins: &Pins,
    stage: Stage,
) -> Result<BTreeMap<String, String>> {
    let mut images = BTreeMap::new();
    let mut services = serde_json::Map::new();
    let (mut joined, declared) = match stage {
        Stage::Checked => Default::default(),
        Stage::Published(checked) => link::join(service, checked)?,
    };
    let mut published = serde_json::Map::new();
    for (name, pin) in pins {
        let image = service
            .images
            .get(name)
            .with_context(|| format!("{} no longer pins images.{name}", service.name))?;
        let mut entry = serde_json::json!({ "image": image.reference(pin) });
        if let Some(networks) = joined.remove(name) {
            entry["networks"] = networks;
        }
        if let Stage::Published(_) = stage {
            entry["ports"] = match port::bind(&image.ports, &places.lookup)? {
                Bound::Ready(entries) => entries.into(),
                Bound::Waiting(interface) => bail!(
                    "{} publishes on {interface}, which has no address yet: apply again once \
                     it has one",
                    service.name
                ),
            };
        }
        images.insert(name.clone(), image.reference(pin));
        services.insert(name.clone(), entry);
        published.insert(name.clone(), serde_json::to_value(&image.ports)?);
    }
    let mut config = serde_json::json!({
        "services": services,
        OWN: { "ports": published },
    });
    if !declared.is_empty() {
        config["networks"] = declared.into();
    }
    write_config(places, &service.name, &config)?;
    Ok(images)
}

pub fn write_config(places: &Places, name: &str, config: &serde_json::Value) -> Result<()> {
    let text = serde_json::to_string_pretty(config)?;
    file::write(&places.project(name).join(OVERRIDE), text.as_bytes(), 0o644)
}

pub enum Binding {
    Current,
    Stale(Value),
    Waiting(String),
}

pub struct Bindings {
    pub binding: Binding,
    pub interfaces: BTreeSet<String>,
}

fn read_override(places: &Places, name: &str) -> Result<(PathBuf, Value)> {
    let path = places.project(name).join(OVERRIDE);
    let text = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let config =
        serde_json::from_slice(&text).with_context(|| format!("reading {}", path.display()))?;
    Ok((path, config))
}

pub fn binding(places: &Places, name: &str) -> Result<Bindings> {
    let (path, mut config) = read_override(places, name)?;
    let own = config
        .get(OWN)
        .with_context(|| format!("{} has no {OWN}", path.display()))?;
    let declared: BTreeMap<String, Vec<Published>> =
        serde_json::from_value(own["ports"].clone())
            .with_context(|| format!("reading {OWN} in {}", path.display()))?;
    let interfaces = declared
        .values()
        .flatten()
        .filter_map(|p| p.network.interface().map(str::to_owned))
        .collect();
    let mut stale = false;
    for (service, ports) in &declared {
        let entries = match port::bind(ports, &places.lookup)? {
            Bound::Ready(entries) => Value::from(entries),
            Bound::Waiting(interface) => {
                return Ok(Bindings {
                    binding: Binding::Waiting(interface),
                    interfaces,
                });
            }
        };
        let current = &mut config["services"][service]["ports"];
        let current_entries = current.as_array().map_or(&[][..], Vec::as_slice);
        if current_entries != entries.as_array().expect("built as an array").as_slice() {
            *current = entries;
            stale = true;
        }
    }
    Ok(Bindings {
        binding: match stale {
            true => Binding::Stale(config),
            false => Binding::Current,
        },
        interfaces,
    })
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

pub fn env(rendered: &Rendered) -> Vec<(String, String)> {
    variables(
        rendered
            .secrets
            .iter()
            .map(|(name, bytes)| (name.clone(), &bytes[..])),
    )
}

pub fn written_env(places: &Places, name: &str) -> Result<Vec<(String, String)>> {
    let dir = places.project(name).join(SECRETS);
    let mut secrets = Vec::new();
    for rel in walk::files(&dir)? {
        let path = dir.join(&rel);
        let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        secrets.push((rel.to_string_lossy().into_owned(), bytes));
    }
    Ok(variables(
        secrets
            .iter()
            .map(|(name, bytes)| (name.clone(), &bytes[..])),
    ))
}

fn variables<'a>(secrets: impl Iterator<Item = (String, &'a [u8])>) -> Vec<(String, String)> {
    secrets
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
    pub overwrite: Vec<PathBuf>,
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
    if service.unbuilt() {
        bail!("{name} waits for the fleet's CI to build its recipe");
    }
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
        if let Some(why) = engine.confirm(&wanted.key, image, &image.pin())? {
            entry.fleet = wanted.fleet.clone();
            entry.problem = Some(Problem {
                fleet: wanted.fleet.clone(),
                why: why.clone(),
            });
            return Ok((entry, Outcome::Refused(why)));
        }
        unconfirmed.confirm(&wanted.key, image_name);
    }

    let dir = places.project(name);
    let previous = places.previous(name);
    let update = !entry.running.is_empty();
    if update {
        keep_copy(&dir, &previous)?;
    }
    let project = Project {
        name,
        dir: dir.clone(),
        env: env(wanted.rendered),
    };
    let (files, data) = match prepare(engine, places, wanted, &entry, &project) {
        Ok(prepared) => prepared,
        Err(e) if update => {
            put_back(&dir, &previous)?;
            return Err(e);
        }
        Err(e) => return Err(e),
    };
    let mut snapshotted = false;
    let started = (|| {
        if update {
            engine.stop(name)?;
            engine.snapshot(name, &data, &service.snapshot)?;
            snapshotted = true;
        }
        join_links(engine, places, name)?;
        engine.up(&project)?;
        engine.health(name)
    })();
    let failure = match started {
        Ok(Health::Healthy) => None,
        Ok(Health::Unhealthy(why)) => Some(why),
        Err(e) => Some(format!("{e:#}")),
    };
    entry.fleet = wanted.fleet.clone();
    let outcome = match failure {
        Some(why) if update => {
            let after = roll_back(
                engine,
                places,
                &project,
                &data,
                &previous,
                snapshotted.then_some(&service.snapshot[..]),
            )
            .with_context(|| format!("{name} failed ({why}), and putting it back failed too"))?;
            engine.drop_snapshot(name)?;
            let failed: Vec<(&String, &super::Pin)> = pins
                .iter()
                .filter(|(image, pin)| entry.running.get(*image) != Some(pin))
                .collect();
            let what = match failed.is_empty() {
                true => "its new files".to_owned(),
                false => describe(&failed),
            };
            let mut problem = format!(
                "{what} failed ({why}), so it runs {} again",
                describe(&entry.running.iter().collect::<Vec<_>>())
            );
            if let Health::Unhealthy(again) = after {
                problem.push_str(&format!(", which isn't healthy either: {again}"));
            }
            entry
                .failed
                .extend(failed.into_iter().map(|(i, p)| (i.clone(), p.clone())));
            entry.problem = Some(Problem {
                fleet: wanted.fleet.clone(),
                why: problem,
            });
            return Ok((entry, Outcome::RolledBack(why)));
        }
        Some(why) => Outcome::Unhealthy(why),
        None if update => Outcome::Updated,
        None => Outcome::Started,
    };
    if update {
        engine.drop_snapshot(name)?;
        fs::remove_dir_all(&previous)
            .with_context(|| format!("removing {}", previous.display()))?;
    }
    entry.files = files;
    entry.running = pins;
    entry.problem = match &outcome {
        Outcome::Unhealthy(why) => Some(Problem {
            fleet: wanted.fleet.clone(),
            why: format!("it isn't healthy: {why}"),
        }),
        _ => None,
    };
    Ok((entry, outcome))
}

fn prepare(
    engine: &dyn Engine,
    places: &Places,
    wanted: &Wanted,
    entry: &Deployed,
    project: &Project,
) -> Result<(BTreeMap<PathBuf, String>, PathBuf)> {
    let service = wanted.service;
    let files = write_files(places, wanted, entry)?;
    let images = write_override(places, service, &service.pins(), Stage::Checked)?;
    let data = data_dir(places, &service.name)?;
    let shown = || format!("checking {}/{COMPOSE}", service.dir.display());
    let written = project.dir.join(COMPOSE);
    let text =
        fs::read_to_string(&written).with_context(|| format!("reading {}", written.display()))?;
    compose::check_written(&text, &project.dir).with_context(shown)?;
    let checked = engine.config(project)?;
    compose::check(&checked, &service.name, &project.dir, &data, &images).with_context(shown)?;
    write_override(places, service, &service.pins(), Stage::Published(&checked))?;
    engine.pull(project)?;
    Ok((files, data))
}

fn keep_copy(dir: &Path, copy: &Path) -> Result<()> {
    if copy.exists() {
        fs::remove_dir_all(copy).with_context(|| format!("removing {}", copy.display()))?;
    }
    let status = std::process::Command::new("cp")
        .arg("-a")
        .arg("--")
        .arg(dir)
        .arg(copy)
        .status()
        .context("running cp")?;
    if !status.success() {
        bail!("couldn't copy {} to {}", dir.display(), copy.display());
    }
    Ok(())
}

fn put_back(dir: &Path, copy: &Path) -> Result<()> {
    fs::remove_dir_all(dir).with_context(|| format!("removing {}", dir.display()))?;
    fs::rename(copy, dir).with_context(|| format!("putting back {}", dir.display()))
}

fn roll_back(
    engine: &dyn Engine,
    places: &Places,
    project: &Project,
    data: &Path,
    previous: &Path,
    snapshot: Option<&[PathBuf]>,
) -> Result<Health> {
    engine.stop(project.name)?;
    if let Some(parts) = snapshot {
        engine.restore(project.name, data, parts)?;
    }
    put_back(&project.dir, previous)?;
    if let Binding::Stale(config) = binding(places, project.name)?.binding {
        write_config(places, project.name, &config)?;
    }
    join_links(engine, places, project.name)?;
    engine.up(&Project {
        name: project.name,
        dir: project.dir.clone(),
        env: written_env(places, project.name)?,
    })?;
    engine.health(project.name)
}

pub fn join_links(engine: &dyn Engine, places: &Places, name: &str) -> Result<()> {
    let (_, config) = read_override(places, name)?;
    for network in config["networks"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(n, _)| n)
    {
        engine.link(network)?;
    }
    Ok(())
}

pub fn describe(pins: &[(&String, &super::Pin)]) -> String {
    pins.iter()
        .map(|(image, pin)| format!("images.{image} {}", pin.version))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn remove(engine: &dyn Engine, places: &Places, name: &str, entry: &Deployed) -> Result<()> {
    engine.down(name)?;
    engine.drop_snapshot(name)?;
    let dir = places.project(name);
    let edited = edited(places, name, entry)?;
    for rel in entry.files.keys().filter(|rel| !edited.contains(rel)) {
        file::remove_if_found(&dir.join(rel))?;
    }
    file::remove_if_found(&dir.join(OVERRIDE))?;
    let previous = places.previous(name);
    if previous.exists() {
        fs::remove_dir_all(&previous)?;
    }
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
#[path = "../../tests/services/deploy.rs"]
mod tests;
