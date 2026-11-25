use super::{
    confirm::Unconfirmed,
    deploy::{self, Binding, Bindings, Places, Wanted, binding},
    docker::{Docker, Engine, Project},
    port, registry,
    state::State,
};
use crate::{fleet::Fleet, util::http::Curl};
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
        .filter(|(name, entry)| !entry.running.is_empty() && entry.update.is_none() && wanted(name))
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

fn waiting(
    engine: &dyn Engine,
    places: &Places,
    state: &mut State,
    facing: Option<&str>,
) -> Result<Vec<String>> {
    let ready: Vec<(String, String)> = state
        .services
        .iter()
        .filter_map(|(name, entry)| {
            let waiting = entry.waiting.as_ref()?;
            facing
                .is_none_or(|interface| interface == waiting.interface)
                .then(|| (name.clone(), waiting.fleet.clone()))
        })
        .collect();
    if ready.is_empty() {
        return Ok(Vec::new());
    }
    let fleet = Fleet::load()?;
    let mut unconfirmed = Unconfirmed::load()?;
    let mut failed = Vec::new();
    for (name, applied) in ready {
        let mut one = || -> Result<()> {
            let service = super::find(&fleet.services, &name)?;
            let rendered = service.render(fleet.root_keys())?;
            let wanted = Wanted::of(&fleet, service, &rendered);
            if wanted.fleet != applied {
                println!("Left {name} alone: the fleet changed since apply, so apply starts it");
                return Ok(());
            }
            deploy::record(engine, places, &wanted, state, &mut unconfirmed)
        };
        if let Err(e) = one() {
            failed.push(format!("{name} didn't start: {e:#}"));
        }
    }
    Ok(failed)
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
    let mut state = State::load()?;
    let mut failed = services(&docker, &places, &state, interface, &|_| true);
    failed.extend(waiting(&docker, &places, &mut state, interface)?);
    if !failed.is_empty() {
        bail!("{}", failed.join("\n"));
    }
    Ok(())
}
