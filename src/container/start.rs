use super::{
    Published,
    deploy::{self, OVERRIDE, OWN, Places},
    docker::{Docker, Engine, Project},
    port::{self, Bound},
    registry,
    state::State,
};
use crate::{fleet::Fleet, http::Curl};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    thread,
    time::{Duration, Instant},
};

const ADDRESS_WAIT: Duration = Duration::from_secs(30);

pub enum Binding {
    Current,
    Stale(Value),
    Waiting(String),
}

pub struct Bindings {
    pub binding: Binding,
    pub interfaces: BTreeSet<String>,
}

pub type Env<'a> = &'a dyn Fn(&str) -> Result<Vec<(String, String)>>;

pub fn binding(places: &Places, name: &str) -> Result<Bindings> {
    let path = places.project(name).join(OVERRIDE);
    let text = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut config: Value =
        serde_json::from_slice(&text).with_context(|| format!("reading {}", path.display()))?;
    let declared: BTreeMap<String, Vec<Published>> =
        serde_json::from_value(config[OWN]["ports"].clone()).unwrap_or_default();
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

pub fn services(
    engine: &dyn Engine,
    places: &Places,
    state: &State,
    facing: Option<&str>,
    wanted: &dyn Fn(&str) -> bool,
    env: Env,
) -> Vec<String> {
    state
        .services
        .iter()
        .filter(|(name, entry)| !entry.running.is_empty() && wanted(name))
        .filter_map(|(name, _)| {
            one(engine, places, name, facing, env)
                .err()
                .map(|e| format!("{name} didn't start: {e:#}"))
        })
        .collect()
}

fn one(
    engine: &dyn Engine,
    places: &Places,
    name: &str,
    facing: Option<&str>,
    env: Env,
) -> Result<()> {
    let Bindings {
        binding,
        interfaces,
    } = binding(places, name)?;
    if facing.is_some_and(|interface| !interfaces.contains(interface)) {
        return Ok(());
    }
    match binding {
        Binding::Waiting(_) => Ok(()),
        Binding::Current => engine.start(name),
        Binding::Stale(config) => {
            let env = env(name)?;
            deploy::write_override_text(places, name, &serde_json::to_string_pretty(&config)?)?;
            engine.up(&Project {
                name,
                dir: places.project(name),
                env,
            })
        }
    }
}

pub fn fleet_env(fleet: &Fleet, name: &str) -> Result<Vec<(String, String)>> {
    let service = fleet
        .services
        .iter()
        .find(|s| s.name == name)
        .with_context(|| format!("the fleet no longer describes {name}: apply removes it"))?;
    Ok(deploy::env(&service.render(fleet.root_keys())?))
}

pub fn command(interface: Option<&str>) -> Result<()> {
    let places = Places::here()?;
    if let Some(interface) = interface {
        let start = Instant::now();
        while (places.lookup)(interface)?.is_empty() && start.elapsed() < ADDRESS_WAIT {
            thread::sleep(Duration::from_secs(1));
        }
    }
    let client = registry::Client::new(Box::new(Curl), None)?;
    let Some(docker) = Docker::find(client)? else {
        return Ok(());
    };
    if !docker.answers() {
        return Ok(());
    }
    let fleet = std::cell::OnceCell::new();
    let env = |name: &str| {
        let fleet = match fleet.get_or_init(Fleet::load) {
            Ok(fleet) => fleet,
            Err(e) => bail!("{e:#}"),
        };
        fleet_env(fleet, name)
    };
    let failed = services(
        &docker,
        &places,
        &State::load()?,
        interface,
        &|_| true,
        &env,
    );
    if !failed.is_empty() {
        bail!("{}", failed.join("\n"));
    }
    Ok(())
}
