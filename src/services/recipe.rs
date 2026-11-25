use super::{DOCKERFILE, Image, Pin, REVISION_LABEL, SERVICES, registry::Client};
use crate::fleet::checked::{self, Git};
use anyhow::Result;

pub fn path(service: &str) -> String {
    let (type_name, name) = service.split_once('/').expect("service keys name both");
    format!("types/{type_name}/{SERVICES}/{name}/{DOCKERFILE}")
}

pub fn built_from(
    git: &Git,
    accepted: &str,
    registry: &Client,
    service: &str,
    image: &Image,
    pin: &Pin,
) -> Result<Option<String>> {
    if !image.recipe {
        return Ok(None);
    }
    let shown = format!("{}:{}", image.image, pin.version);
    let labels = registry.labels(&image.image, &pin.digest)?;
    let Some(commit) = labels.get(REVISION_LABEL) else {
        return Ok(Some(format!("{shown} names no {REVISION_LABEL}")));
    };
    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(Some(format!(
            "{shown} names {commit} as its commit, which isn't one"
        )));
    }
    if !git.is_ancestor(commit, accepted)? {
        return Ok(Some(format!(
            "{shown} was built from {}, which isn't on the fleet's main as machines accept it",
            checked::short(commit)
        )));
    }
    let path = path(service);
    if git.blob(commit, &path)? != git.blob(accepted, &path)? {
        return Ok(Some(format!(
            "{shown} was built from {}, whose {path} isn't the current one",
            checked::short(commit)
        )));
    }
    Ok(None)
}

#[cfg(test)]
#[path = "../../tests/services/recipe.rs"]
mod tests;
