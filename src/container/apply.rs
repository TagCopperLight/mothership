use super::{
    account,
    confirm::Unconfirmed,
    deploy::{self, Outcome, Places, Wanted},
    docker::Docker,
    docker_config, registry,
    state::State,
};
use crate::{file, fleet::Fleet, http::Curl, probe};
use anyhow::{Context, Result, bail};
use std::path::PathBuf;

pub enum Action {
    Setup,
    Deploy(String),
    Remove(String),
    RemoveContainer(String),
    Overwrite { service: String, file: PathBuf },
    Probe,
    Unprobe,
}

impl Action {
    pub fn describe(&self) -> &'static str {
        match self {
            Action::Setup => "set it up",
            Action::Deploy(_) => "start the fleet's versions",
            Action::Remove(_) => "stop it and remove its containers, keeping its data",
            Action::RemoveContainer(_) => "remove it",
            Action::Overwrite { .. } => "write the fleet's version",
            Action::Probe => "install and start it",
            Action::Unprobe => "stop and remove it",
        }
    }
}

const DOCKER_CONFIG: &str = ".docker/config.json";

pub fn run(fleet: &Fleet, actions: &[&Action]) -> Result<()> {
    for action in actions {
        match action {
            Action::Setup => account::setup(&super::published(&fleet.services)?)?,
            Action::Probe => probe::unit::install(fleet)?,
            Action::Unprobe => probe::unit::remove()?,
            _ => {}
        }
    }
    if actions
        .iter()
        .all(|a| matches!(a, Action::Setup | Action::Probe | Action::Unprobe))
    {
        return Ok(());
    }
    let logins = docker_config(fleet)?;
    let token = super::token(fleet)?;
    let client = registry::Client::new(Box::new(Curl), logins.as_deref())?;
    let docker = Docker::find(client)?.context("the services account is missing")?;
    let config = docker.user.home.join(DOCKER_CONFIG);
    docker.user.with_rights(|| match &logins {
        Some(bytes) => file::write(&config, bytes, 0o600),
        None => file::remove_if_found(&config).map(drop),
    })?;
    let places = Places::here()?;
    let mut state = State::load()?;
    let mut unconfirmed = Unconfirmed::load()?;
    let mut deploys: Vec<&str> = Vec::new();
    let mut overwrite: Vec<(&str, PathBuf)> = Vec::new();
    for action in actions {
        match action {
            Action::Setup | Action::Probe | Action::Unprobe => {}
            Action::RemoveContainer(id) => docker.remove_container(id)?,
            Action::Remove(name) => {
                let entry = state
                    .services
                    .get(name)
                    .with_context(|| format!("this machine runs no service {name}"))?;
                deploy::remove(&docker, &places, name, entry)?;
                state.services.remove(name);
                state.save()?;
                println!("Removed {name}, keeping {}", places.data(name).display());
            }
            Action::Overwrite { service, file } => {
                overwrite.push((service, file.clone()));
                deploys.push(service);
            }
            Action::Deploy(name) => deploys.push(name),
        }
    }
    deploys.sort();
    deploys.dedup();
    let mut failed = Vec::new();
    for name in deploys {
        let service = fleet
            .services
            .iter()
            .find(|s| s.name == name)
            .with_context(|| format!("the fleet has no service {name}"))?;
        let rendered = service.render(fleet.root_keys())?;
        let wanted = Wanted {
            service,
            key: super::key(&fleet.type_name, service),
            fleet: deploy::fingerprint(service, &rendered, token.as_deref()),
            rendered: &rendered,
            overwrite: overwrite
                .iter()
                .filter(|(owner, _)| *owner == name)
                .map(|(_, file)| file.clone())
                .collect(),
        };
        let before = state.services.get(name);
        match deploy::deploy(&docker, &places, &wanted, before, &mut unconfirmed) {
            Ok((entry, outcome)) => {
                println!("{}", crate::style::clean(&said(name, &outcome)));
                state.services.insert(name.to_owned(), entry);
            }
            Err(e) => failed.push(format!("{name}: {e:#}")),
        }
        state.save()?;
        unconfirmed.save()?;
    }
    if !failed.is_empty() {
        bail!("couldn't apply {}", failed.join("\n"));
    }
    Ok(())
}

fn said(name: &str, outcome: &Outcome) -> String {
    match outcome {
        Outcome::Started => format!("Started {name}"),
        Outcome::Updated => format!("Updated {name}"),
        Outcome::Unhealthy(why) => format!("Started {name}, which isn't healthy: {why}"),
        Outcome::RolledBack(why) => format!(
            "{name} failed its health check, so it runs its last healthy versions again: {why}"
        ),
        Outcome::Blocked(pins) => {
            format!("Left {name} alone: {pins} failed its health check here before")
        }
        Outcome::Refused(why) => format!("Left {name} alone: {why}"),
    }
}

pub fn problems() -> Result<Vec<String>> {
    Ok(State::load()?
        .services
        .iter()
        .filter_map(|(name, entry)| Some(format!("{name}: {}", entry.problem_for(&entry.fleet)?)))
        .collect())
}
