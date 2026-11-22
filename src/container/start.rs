use super::{
    deploy::{self, Binding, Bindings, Places, binding},
    docker::{Docker, Engine, Project},
    port, registry,
    state::State,
};
use crate::{fleet::Fleet, http::Curl};
use anyhow::{Context, Result, bail};
use std::{
    thread,
    time::{Duration, Instant},
};

const ADDRESS_WAIT: Duration = Duration::from_secs(30);

pub type Env<'a> = &'a dyn Fn(&str) -> Result<Vec<(String, String)>>;

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
            deploy::write_config(places, name, &config)?;
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

pub fn wait_for_address(interface: &str) -> Result<()> {
    let start = Instant::now();
    while port::addresses(interface)?.is_empty() && start.elapsed() < ADDRESS_WAIT {
        thread::sleep(Duration::from_secs(1));
    }
    Ok(())
}

pub fn command(interface: Option<&str>) -> Result<()> {
    let places = Places::here()?;
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
