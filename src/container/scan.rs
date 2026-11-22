use super::{
    Action, Service, WORKFLOW, WORKFLOW_PATH, account,
    deploy::{self, Places},
    docker::Docker,
    state::{Deployed, State},
};
use crate::{
    edit::Add,
    file,
    fleet::Fleet,
    item::{Change, Item, Section},
    kind::Kind,
    preview::Preview,
    probe, walk,
};
use anyhow::Result;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub fn scan(fleet: &Fleet, docker: Option<&Docker>) -> Result<Vec<Item>> {
    let mut items = probe::unit::items(fleet)?;
    items.extend(workflow(&fleet.dir)?);
    let state = State::load()?;
    let account = account::user()?;
    if fleet.services.is_empty() && account.is_none() {
        return Ok(items);
    }
    let places = Places::here()?;
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
    let owned: BTreeSet<&str> = fleet
        .services
        .iter()
        .map(|s| s.name.as_str())
        .chain(state.services.keys().map(String::as_str))
        .collect();
    for left in unowned(&places, &owned)? {
        items.push(
            Item::new(
                Kind::Data,
                Change::Added,
                left.display().to_string(),
                "no service",
            )
            .noting("no service of the fleet owns it")
            .because("it holds what the machine produced: delete it yourself once done"),
        );
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

pub fn workflow(fleet_dir: &Path) -> Result<Option<Item>> {
    if !super::holds_recipes(fleet_dir)? {
        return Ok(None);
    }
    let detail = match file::read_if_any(&fleet_dir.join(WORKFLOW_PATH))? {
        Some(text) if text == WORKFLOW => return Ok(None),
        Some(_) => "outdated",
        None => "missing",
    };
    Ok(Some(
        Item::new(Kind::Container, Change::Edited, WORKFLOW_PATH, detail)
            .noting("the fleet holds recipes, and this is the workflow that builds them")
            .with(Add::WriteWorkflow),
    ))
}

fn unowned(places: &Places, owned: &BTreeSet<&str>) -> Result<Vec<PathBuf>> {
    let mut left = Vec::new();
    for dir in [&places.data, &places.projects] {
        for path in walk::children(dir)? {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            let kept = name.as_deref().is_some_and(|name| {
                owned.contains(name)
                    || owned
                        .iter()
                        .any(|owner| places.previous(owner).file_name() == Some(name.as_ref()))
            });
            if !kept {
                left.push(path);
            }
        }
    }
    Ok(left)
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
    let sealed = !service.secret_files()?.is_empty() || super::token_path(fleet).exists();
    if sealed && !fleet.etc_open() {
        return Ok(vec![Item::locked(Kind::Container, &service.name)]);
    }
    if service.unbuilt() {
        return Ok(vec![
            Item::new(
                Kind::Container,
                Change::Unreadable,
                &service.name,
                "not built yet",
            )
            .noting("the fleet's CI builds its recipe, and the probe then pins it")
            .because("check the fleet's Recipes workflow if this lasts"),
        ]);
    }
    let rendered = service.render(fleet.root_keys())?;
    let fingerprint = deploy::fingerprint(service, &rendered, super::token(fleet)?.as_deref());
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

#[cfg(test)]
#[path = "../../tests/container_scan.rs"]
mod tests;
