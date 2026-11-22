use crate::file;
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub fn check(
    config: &Value,
    project: &Path,
    data: &Path,
    images: &BTreeMap<String, String>,
) -> Result<()> {
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
    let inside = |source: &Path| {
        file::plain_within(project, source).is_some() || file::plain_within(data, source).is_some()
    };
    for (name, service) in services {
        if service.get("build").is_some() {
            bail!("{name} builds its image: the fleet runs only pinned images");
        }
        if service["image"].as_str() != Some(&images[name]) {
            bail!("{name} doesn't run the image service.toml pins");
        }
        if service.get("ports").is_some() {
            bail!("{name} publishes ports in compose.yaml: declare them in service.toml");
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
            if let Some(source) = entry["file"].as_str()
                && file::plain_within(project, Path::new(source)).is_none()
            {
                bail!("the {kind} entry {name} reads {source}, outside the service");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/compose.rs"]
mod tests;
