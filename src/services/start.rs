use super::{
    deploy::{self, Binding, Bindings, Places, binding},
    docker::{Docker, Engine, Project},
    port, registry,
    state::State,
};
use crate::util::http::Curl;
use anyhow::{Result, bail};
use std::{thread, time::Duration};

pub fn services(
    engine: &dyn Engine,
    places: &Places,
    state: &State,
    facing: Option<&str>,
    wanted: &dyn Fn(&str) -> bool,
) -> Vec<String> {
    state
        .services
        .iter()
        .filter(|(name, entry)| !entry.running.is_empty() && wanted(name))
        .filter_map(|(name, _)| {
            one(engine, places, name, facing)
                .err()
                .map(|e| format!("{name} didn't start: {e:#}"))
        })
        .collect()
}

fn one(engine: &dyn Engine, places: &Places, name: &str, facing: Option<&str>) -> Result<()> {
    let Bindings {
        binding,
        interfaces,
    } = binding(places, name)?;
    if facing.is_some_and(|interface| !interfaces.contains(interface)) {
        return Ok(());
    }
    let stale = match binding {
        Binding::Waiting(_) => return Ok(()),
        Binding::Current => None,
        Binding::Stale(config) => Some(config),
    };
    deploy::join_links(engine, places, name)?;
    match stale {
        None => engine.start(name),
        Some(config) => {
            let env = deploy::written_env(places, name)?;
            deploy::write_config(places, name, &config)?;
            engine.up(&Project {
                name,
                dir: places.project(name),
                env,
            })
        }
    }
}

pub fn wait_for_address(interface: &str) -> Result<()> {
    while port::addresses(interface)?.is_empty() {
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
    let failed = services(&docker, &places, &State::load()?, interface, &|_| true);
    if !failed.is_empty() {
        bail!("{}", failed.join("\n"));
    }
    Ok(())
}
