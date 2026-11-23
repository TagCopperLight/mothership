use super::Service;
use crate::system::host;
use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

pub const PREFIX: &str = "link.";

#[derive(Clone)]
pub struct Link {
    pub service: String,
    pub container: String,
    pub caller: String,
    pub caller_container: String,
}

impl Link {
    pub fn network(&self) -> String {
        format!(
            "{PREFIX}{}.{}.{}.{}",
            self.service, self.container, self.caller, self.caller_container
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
        let (service, container) = split(caller)?;
        for part in [service, container] {
            host::check(part).with_context(|| format!("the caller {caller}"))?;
        }
    }
    Ok(())
}

fn split(caller: &str) -> Result<(&str, &str)> {
    caller.split_once('/').with_context(|| {
        format!("the caller {caller} names no container: write {caller}/<container>")
    })
}

pub fn between(services: &[Service]) -> Result<Vec<Link>> {
    let mut links = Vec::new();
    for service in services {
        for (container, image) in &service.images {
            for entry in &image.callers {
                let (caller, caller_container) = split(entry)?;
                let owner = format!("{}'s images.{container}", service.name);
                if caller == service.name {
                    bail!(
                        "{owner} lets its own service call it: a service's containers reach \
                         each other already"
                    );
                }
                let Some(calling) = services.iter().find(|s| s.name == caller) else {
                    bail!("{owner} lets {caller} call it, and the type has no service {caller}");
                };
                if !calling.images.contains_key(caller_container) {
                    bail!("{owner} lets {entry} call it, and {caller} runs no {caller_container}");
                }
                links.push(Link {
                    service: service.name.clone(),
                    container: container.clone(),
                    caller: caller.to_owned(),
                    caller_container: caller_container.to_owned(),
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
                if link.caller_container == *container {
                    added.insert(link.network(), Value::Null);
                }
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
#[path = "../../tests/services/link.rs"]
mod tests;
