use super::link;
use crate::util::file;
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const NAMESPACES: [&str; 5] = ["pid", "ipc", "uts", "userns_mode", "cgroup"];
const TOP_KEYS: [&str; 6] = [
    "name", "services", "networks", "volumes", "secrets", "configs",
];
const ENTRY_KEYS: [&str; 4] = ["name", "file", "environment", "content"];
const SERVICE_KEYS: [&str; 39] = [
    "image",
    "command",
    "entrypoint",
    "environment",
    "env_file",
    "labels",
    "healthcheck",
    "depends_on",
    "restart",
    "user",
    "working_dir",
    "devices",
    "volumes",
    "tmpfs",
    "networks",
    "secrets",
    "configs",
    "security_opt",
    "privileged",
    "cap_drop",
    "read_only",
    "init",
    "stop_signal",
    "stop_grace_period",
    "hostname",
    "extra_hosts",
    "expose",
    "shm_size",
    "mem_limit",
    "cpus",
    "pids_limit",
    "ulimits",
    "tty",
    "stdin_open",
    "pid",
    "ipc",
    "uts",
    "userns_mode",
    "cgroup",
];
const REFUSED: [(&str, &str); 6] = [
    (
        "build",
        "builds its image: the fleet runs only pinned images",
    ),
    (
        "ports",
        "publishes ports in compose.yaml: declare them in service.toml",
    ),
    (
        "network_mode",
        "sets network_mode: a container stays on its service's networks",
    ),
    (
        "external_links",
        "links containers outside the service: another service calls it through callers in \
         service.toml",
    ),
    (
        "volumes_from",
        "mounts another container's volumes: a service reaches only its own files",
    ),
    (
        "cap_add",
        "adds capabilities: a container keeps Docker's default rights",
    ),
];

pub fn check_written(text: &str, project: &Path) -> Result<()> {
    let mut yaml: yaml_serde::Value = yaml_serde::from_str(text).context("reading it")?;
    yaml.apply_merge().context("reading it")?;
    let config = serde_json::to_value(yaml).context("reading it")?;
    check_keys(&config)?;
    for (name, service) in config["services"].as_object().into_iter().flatten() {
        let files = match &service["env_file"] {
            Value::Null => Vec::new(),
            Value::Array(files) => files.iter().collect(),
            file => vec![file],
        };
        for file in files {
            let path = file
                .as_str()
                .or_else(|| file["path"].as_str())
                .unwrap_or("");
            if path.contains('$') || path.starts_with('~') {
                bail!("{name} reads {path}: name an env_file without variables");
            }
            in_project(project, &project.join(path), name)?;
        }
    }
    Ok(())
}

fn check_keys(config: &Value) -> Result<()> {
    known("compose.yaml", config, &TOP_KEYS)?;
    for (name, service) in config["services"].as_object().into_iter().flatten() {
        known(name, service, &SERVICE_KEYS)?;
    }
    for kind in ["secrets", "configs"] {
        for (name, entry) in config[kind].as_object().into_iter().flatten() {
            known(&format!("the {kind} entry {name}"), entry, &ENTRY_KEYS)?;
        }
    }
    Ok(())
}

fn known(owner: &str, entry: &Value, allowed: &[&str]) -> Result<()> {
    for key in entry.as_object().into_iter().flatten().map(|(key, _)| key) {
        if key.starts_with("x-") || allowed.contains(&key.as_str()) {
            continue;
        }
        if let Some((_, why)) = REFUSED.iter().find(|(refused, _)| refused == key) {
            bail!("{owner} {why}");
        }
        bail!("{owner} sets {key}, which mothership doesn't accept");
    }
    Ok(())
}

fn in_project(project: &Path, source: &Path, owner: &str) -> Result<()> {
    if file::plain_within(project, source).is_none() {
        bail!("{owner} reads {}, outside the service", source.display());
    }
    Ok(())
}

pub fn check(
    config: &Value,
    name: &str,
    project: &Path,
    data: &Path,
    images: &BTreeMap<String, String>,
) -> Result<()> {
    check_keys(config)?;
    let services = config["services"]
        .as_object()
        .context("docker compose config printed no services")?;
    let described: BTreeSet<&String> = services.keys().collect();
    let pinned: BTreeSet<&String> = images.keys().collect();
    if let Some(name) = described.difference(&pinned).next() {
        bail!("compose.yaml runs {name}, which service.toml pins no image for");
    }
    if let Some(name) = pinned.difference(&described).next() {
        bail!("service.toml pins images.{name}, which compose.yaml doesn't run");
    }
    if config["volumes"].as_object().is_some_and(|v| !v.is_empty()) {
        bail!(
            "compose.yaml declares Docker volumes: keep data in {}",
            data.display()
        );
    }
    for (key, network) in config["networks"].as_object().into_iter().flatten() {
        if key.starts_with(link::PREFIX) {
            bail!(
                "compose.yaml names a network {key}: names starting with {} are mothership's links",
                link::PREFIX
            );
        }
        if network["external"].as_bool() == Some(true)
            || network["name"].as_str() != Some(&format!("{name}_{key}"))
        {
            bail!(
                "compose.yaml uses the network {key} from outside the service: another service \
                 calls it through callers in service.toml"
            );
        }
    }
    let inside = |source: &Path| {
        file::plain_within(project, source).is_some() || file::plain_within(data, source).is_some()
    };
    for (name, service) in services {
        for key in NAMESPACES {
            let shared = service[key]
                .as_str()
                .is_some_and(|value| value == "host" || value.starts_with("container:"));
            if shared {
                bail!("{name} shares its {key} namespace outside the service");
            }
        }
        if service["privileged"].as_bool() == Some(true) {
            bail!("{name} runs privileged: a container keeps Docker's default rights");
        }
        for option in service["security_opt"].as_array().into_iter().flatten() {
            let option = option.as_str().unwrap_or("");
            if option.split([':', '=']).next() != Some("no-new-privileges") {
                bail!("{name} sets security_opt {option}: only no-new-privileges is allowed");
            }
        }
        if service["image"].as_str() != Some(&images[name]) {
            bail!("{name} doesn't run the image service.toml pins");
        }
        for volume in service["volumes"].as_array().into_iter().flatten() {
            let source = volume["source"].as_str().unwrap_or("");
            match volume["type"].as_str() {
                Some("tmpfs") => {}
                Some("bind") if inside(Path::new(source)) => {}
                Some("bind") => bail!(
                    "{name} mounts {source}: a service reaches only {} and its own files",
                    data.display()
                ),
                _ => bail!(
                    "{name} uses a Docker volume: keep its data in {}",
                    data.display()
                ),
            }
        }
    }
    for kind in ["secrets", "configs"] {
        for (name, entry) in config[kind].as_object().into_iter().flatten() {
            if let Some(source) = entry["file"].as_str() {
                in_project(
                    project,
                    Path::new(source),
                    &format!("the {kind} entry {name}"),
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/services/compose.rs"]
mod tests;
