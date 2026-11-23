use super::{
    account,
    confirm::Unconfirmed,
    deploy::{self, Outcome, Places, Wanted},
    docker::Docker,
    docker_config, registry, start,
    state::State,
};
use crate::{fleet::Fleet, probe, util::file, util::http::Curl};
use anyhow::{Context, Result, bail};
use std::path::PathBuf;

pub enum Action {
    Setup,
    Teardown,
    Deploy(String),
    Remove(String),
    Start(String),
    RemoveContainer(String),
    RemoveLink(String),
    Overwrite { service: String, file: PathBuf },
    Probe,
    Unprobe,
    Login,
}

impl Action {
    pub fn takes_away(&self) -> bool {
        matches!(
            self,
            Action::Teardown
                | Action::Remove(_)
                | Action::RemoveContainer(_)
                | Action::RemoveLink(_)
        )
    }

    pub fn describe(&self) -> &'static str {
        match self {
            Action::Setup => "set it up",
            Action::Teardown => "remove it and everything it runs",
            Action::Deploy(_) => "start the fleet's versions",
            Action::Remove(_) => "stop it and remove its containers, keeping its data",
            Action::Start(_) => "start it",
            Action::RemoveContainer(_) => "remove it",
            Action::RemoveLink(_) => "disconnect its containers and remove it",
            Action::Overwrite { .. } => "write the fleet's version",
            Action::Probe => "install and start it",
            Action::Unprobe => "stop and remove it",
            Action::Login => "write the fleet's login",
        }
    }
}

pub fn run(fleet: &Fleet, actions: &[&Action]) -> Result<()> {
    for action in actions {
        match action {
            Action::Setup => account::setup(&fleet.services)?,
            Action::Probe => probe::unit::install(fleet)?,
            Action::Unprobe => probe::unit::remove()?,
            _ => {}
        }
    }
    if actions.iter().any(|a| matches!(a, Action::Teardown)) {
        return teardown(fleet);
    }
    let set_up = actions.iter().any(|a| matches!(a, Action::Setup));
    if !set_up
        && actions
            .iter()
            .all(|a| matches!(a, Action::Probe | Action::Unprobe))
    {
        return Ok(());
    }
    let logins = docker_config(fleet)?;
    let client = registry::Client::new(Box::new(Curl), logins.as_deref())?;
    let docker = Docker::find(client)?.context("the services account is missing")?;
    let config = account::logins(&docker.user);
    docker.user.with_rights(|| match &logins {
        Some(bytes) => file::write(&config, bytes, 0o600),
        None => file::remove_if_found(&config).map(drop),
    })?;
    let places = Places::here()?;
    let mut state = State::load()?;
    let mut unconfirmed = Unconfirmed::load()?;
    let mut failed = Vec::new();
    let busy: Vec<&str> = actions
        .iter()
        .filter_map(|action| match action {
            Action::Deploy(name) | Action::Remove(name) => Some(name.as_str()),
            Action::Overwrite { service, .. } => Some(service.as_str()),
            _ => None,
        })
        .collect();
    let bind: Vec<&str> = actions
        .iter()
        .filter_map(|action| match action {
            Action::Start(name) => Some(name.as_str()),
            _ => None,
        })
        .collect();
    let wanted = |name: &str| !busy.contains(&name) && (set_up || bind.contains(&name));
    failed.extend(start::services(&docker, &places, &state, None, &wanted));
    let mut deploys: Vec<&str> = Vec::new();
    let mut overwrite: Vec<(&str, PathBuf)> = Vec::new();
    for action in actions {
        match action {
            Action::Setup
            | Action::Teardown
            | Action::Probe
            | Action::Unprobe
            | Action::Login
            | Action::Start(_) => {}
            Action::RemoveContainer(id) => docker.remove_container(id)?,
            Action::RemoveLink(network) => docker.remove_link(network)?,
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
            fleet: deploy::fingerprint(service, &rendered),
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
                println!("{}", crate::util::style::clean(&said(name, &outcome)));
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

fn teardown(fleet: &Fleet) -> Result<()> {
    let client = registry::Client::new(Box::new(Curl), None)?;
    let mut state = State::load()?;
    if let Some(docker) = Docker::find(client)? {
        let places = Places::here()?;
        for (name, entry) in state.services.clone() {
            deploy::remove(&docker, &places, &name, &entry)?;
            state.services.remove(&name);
            state.save()?;
            println!("Removed {name}, keeping {}", places.data(&name).display());
        }
    }
    account::teardown()?;
    println!(
        "Removed the {} account: {} runs no service",
        account::USER,
        fleet.type_name
    );
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
    let places = Places::here()?;
    let mut problems = Vec::new();
    for (name, entry) in &State::load()?.services {
        if let Some(why) = entry.problem_for(&entry.fleet) {
            problems.push(format!("{name}: {why}"));
        }
        if entry.running.is_empty() {
            continue;
        }
        if let deploy::Binding::Waiting(interface) = deploy::binding(&places, name)?.binding {
            problems.push(format!(
                "{name}: it waits for {interface} to have an address"
            ));
        }
    }
    Ok(problems)
}
