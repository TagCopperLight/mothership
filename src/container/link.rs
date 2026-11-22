use super::Service;
use crate::host;
use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

pub const PREFIX: &str = "link.";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Link {
    pub service: String,
    pub container: String,
    pub caller: String,
}

impl Link {
    pub fn network(&self) -> String {
        format!(
            "{PREFIX}{}.{}.{}",
            self.service, self.container, self.caller
        )
    }

    pub fn name(&self) -> String {
        format!("{}.{}", self.container, self.service)
    }
}

pub fn check_callers(container: &str, callers: &BTreeSet<String>) -> Result<()> {
    if callers.is_empty() {
        return Ok(());
    }
    host::check(container).context("a container that other services call needs a hostname")?;
    for caller in callers {
        host::check(caller).with_context(|| format!("the caller {caller}"))?;
    }
    Ok(())
}

pub fn between(services: &[Service]) -> Result<Vec<Link>> {
    let names: BTreeSet<&str> = services.iter().map(|s| s.name.as_str()).collect();
    let mut links = Vec::new();
    for service in services {
        for (container, image) in &service.images {
            for caller in &image.callers {
                let owner = format!("{}'s images.{container}", service.name);
                if *caller == service.name {
                    bail!(
                        "{owner} lets its own service call it: a service's containers reach \
                         each other already"
                    );
                }
                if !names.contains(caller.as_str()) {
                    bail!("{owner} lets {caller} call it, and the type has no service {caller}");
                }
                links.push(Link {
                    service: service.name.clone(),
                    container: container.clone(),
                    caller: caller.clone(),
                });
            }
        }
    }
    Ok(links)
}

pub fn networks(services: &[Service]) -> BTreeSet<String> {
    services
        .iter()
        .flat_map(|service| &service.links)
        .map(Link::network)
        .collect()
}

pub fn join(service: &Service, config: &Value) -> Result<(Map<String, Value>, Map<String, Value>)> {
    let mut joined = Map::new();
    let mut declared = Map::new();
    for container in service.images.keys() {
        let mut added = Map::new();
        for link in &service.links {
            if link.caller == service.name {
                added.insert(link.network(), Value::Null);
            } else if link.container == *container {
                added.insert(link.network(), json!({ "aliases": [link.name()] }));
            }
        }
        if added.is_empty() {
            continue;
        }
        let mut networks = config["services"][container]["networks"]
            .as_object()
            .cloned()
            .with_context(|| format!("docker compose config gives {container} no network"))?;
        for (network, entry) in added {
            declared.insert(
                network.clone(),
                json!({ "name": network, "external": true }),
            );
            networks.insert(network, entry);
        }
        joined.insert(container.clone(), Value::Object(networks));
    }
    Ok((joined, declared))
}

#[cfg(test)]
#[path = "../../tests/link.rs"]
mod tests;
