use super::{
    Action, Service, account,
    deploy::{self, Places},
    docker::Docker,
    state::{Deployed, State},
};
use crate::{
    fleet::Fleet,
    item::{Change, Item, Section},
    kind::Kind,
    preview::Preview,
};
use anyhow::Result;
use std::collections::BTreeSet;

pub fn scan(fleet: &Fleet, docker: Option<&Docker>) -> Result<Vec<Item>> {
    let state = State::load()?;
    if fleet.services.is_empty() && state.services.is_empty() && state.dropped.is_empty() {
        return Ok(Vec::new());
    }
    let places = Places::here()?;
    let mut items = Vec::new();
    let missing = account::missing()?;
    if !fleet.services.is_empty() && !missing.is_empty() {
        items.push(fleet_item(
            Item::new(Kind::Container, Change::Removed, account::USER, "account")
                .noting(missing.join(", "))
                .reverting(Action::Setup),
        ));
    }
    for service in &fleet.services {
        items.extend(service_items(
            fleet,
            &places,
            service,
            state.services.get(&service.name),
        )?);
    }
    for (name, entry) in &state.services {
        if fleet.services.iter().any(|s| &s.name == name) {
            continue;
        }
        items.push(fleet_item(
            Item::new(Kind::Container, Change::Added, name, "dropped")
                .noting(format!("its data stays in {}", places.data(name).display()))
                .reverting(Action::Remove(name.clone())),
        ));
        items.extend(edited(&places, name, entry)?);
    }
    for name in &state.dropped {
        let (data, project) = (places.data(name), places.project(name));
        for left in [data, project].into_iter().filter(|p| p.exists()) {
            items.push(
                Item::new(
                    Kind::Data,
                    Change::Added,
                    left.display().to_string(),
                    "left",
                )
                .noting(format!("{name} left it, and no service uses it"))
                .because("it holds what the machine produced: delete it yourself once done"),
            );
        }
    }
    match docker {
        Some(docker) if docker.answers() => items.extend(containers(fleet, &state, docker)?),
        Some(_) if missing.is_empty() => items.push(
            Item::new(
                Kind::Container,
                Change::Unreadable,
                "containers",
                "not scanned",
            )
            .noting("the services account's Docker doesn't answer"),
        ),
        _ => {}
    }
    Ok(items)
}

fn fleet_item(mut item: Item) -> Item {
    item.section = Section::Fleet;
    item
}

fn service_items(
    fleet: &Fleet,
    places: &Places,
    service: &Service,
    entry: Option<&Deployed>,
) -> Result<Vec<Item>> {
    let sealed = !service.secret_files()?.is_empty() || super::logins_path(fleet).exists();
    if sealed && !fleet.etc_open() {
        return Ok(vec![Item::locked(Kind::Container, &service.name)]);
    }
    let rendered = service.render(fleet.root_keys())?;
    let fingerprint = deploy::fingerprint(service, &rendered, super::logins(fleet)?.as_deref());
    let pins = service.pins();
    let Some(entry) = entry else {
        return Ok(vec![fleet_item(
            Item::new(Kind::Container, Change::Removed, &service.name, "new")
                .reverting(Action::Deploy(service.name.clone())),
        )]);
    };
    let mut items = edited(places, &service.name, entry)?;
    if entry.fleet != fingerprint {
        let changes: Vec<String> = pins
            .iter()
            .filter(|(image, pin)| entry.running.get(*image) != Some(pin))
            .map(|(image, pin)| format!("{image} {}", pin.version))
            .collect();
        let note = match changes.is_empty() {
            true => "its files changed".to_owned(),
            false => format!("to {}", changes.join(", ")),
        };
        items.push(fleet_item(
            Item::new(Kind::Container, Change::Edited, &service.name, "changed")
                .noting(note)
                .reverting(Action::Deploy(service.name.clone())),
        ));
    } else if let Some(why) = entry.problem_for(&fingerprint) {
        items.push(
            Item::new(
                Kind::Container,
                Change::Unreadable,
                &service.name,
                "not applied",
            )
            .noting(why)
            .because("release a fix or revert the fleet's commit"),
        );
    }
    Ok(items)
}

fn edited(places: &Places, name: &str, entry: &Deployed) -> Result<Vec<Item>> {
    let dir = places.project(name);
    Ok(deploy::edited(places, name, entry)?
        .into_iter()
        .map(|rel| {
            let path = dir.join(&rel);
            Item::new(
                Kind::Container,
                Change::Edited,
                path.display().to_string(),
                "edited",
            )
            .noting(format!("a file of {name}"))
            .showing(Preview::Contents(path))
            .reverting(Action::Overwrite {
                service: name.to_owned(),
                file: rel,
            })
            .because("a service's files come from its folder in the fleet: edit them there")
        })
        .collect())
}

fn containers(fleet: &Fleet, state: &State, docker: &Docker) -> Result<Vec<Item>> {
    let known: BTreeSet<&str> = fleet
        .services
        .iter()
        .map(|s| s.name.as_str())
        .chain(state.services.keys().map(String::as_str))
        .collect();
    let mut items = Vec::new();
    for container in docker.containers()? {
        let shown = container.shown().to_owned();
        let project = container.project().filter(|p| known.contains(p));
        let Some(project) = project else {
            items.push(
                Item::new(Kind::Container, Change::Added, &shown, "not in the fleet")
                    .noting("no service folder describes it")
                    .reverting(Action::RemoveContainer(container.id.clone()))
                    .because("write a service folder for it in the fleet"),
            );
            continue;
        };
        let Some(service) = fleet.services.iter().find(|s| s.name == project) else {
            continue;
        };
        let Some(entry) = state.services.get(project) else {
            continue;
        };
        let compose_name = container
            .config
            .labels
            .as_ref()
            .and_then(|labels| labels.get("com.docker.compose.service"));
        let Some((image, pin)) = compose_name
            .and_then(|name| Some((service.images.get(name)?, entry.running.get(name)?)))
        else {
            continue;
        };
        let wanted = docker.image_id(&image.reference(pin))?;
        if wanted.as_deref() != Some(container.image.as_str()) {
            items.push(
                Item::new(Kind::Container, Change::Edited, &shown, "another image")
                    .noting(format!("{project} pins {}", image.reference(pin)))
                    .reverting(Action::Deploy(project.to_owned()))
                    .because("the probe or a fleet commit changes what a service runs"),
            );
        }
    }
    Ok(items)
}
