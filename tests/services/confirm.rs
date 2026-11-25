use super::*;
use crate::services::{
    load,
    registry::tests::{Fake, response},
    tests::{DIGEST, toml},
};
use std::fs;

const OTHER: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";

fn registry(digest: &'static str) -> Client {
    let fake = Fake {
        sent: Default::default(),
        answer: Box::new(move |_, _, _| response(200, &[("Docker-Content-Digest", digest)], "")),
    };
    Client::new(Box::new(fake), None).unwrap()
}

fn services(dir: &std::path::Path) -> Vec<Service> {
    let svc = dir.join("services/app");
    fs::create_dir_all(&svc).unwrap();
    fs::write(svc.join("service.toml"), toml("1.0.0")).unwrap();
    fs::write(svc.join("compose.yaml"), "services: {app: {}}\n").unwrap();
    load(dir).unwrap()
}

fn accepted(_: &str, _: &Image, _: &Pin) -> Result<Option<String>> {
    Ok(None)
}

fn waiting() -> Unconfirmed {
    let mut unconfirmed = Unconfirmed::default();
    let pin = Pin {
        version: "1.0.0".into(),
        digest: DIGEST.into(),
    };
    unconfirmed.note("server/app", vec![("app".into(), pin)], true);
    unconfirmed
}

#[test]
fn a_pin_the_registry_agrees_with_runs_without_a_question() {
    let dir = tempfile::tempdir().unwrap();
    let mut unconfirmed = waiting();
    let mut asked = Vec::new();
    let mut ask = |q: &str| {
        asked.push(q.to_owned());
        Ok(false)
    };
    ask_republished(
        "server",
        &services(dir.path()),
        &registry(DIGEST),
        &accepted,
        &mut unconfirmed,
        &mut ask,
    )
    .unwrap();
    assert!(asked.is_empty());
    assert_eq!(unconfirmed, Unconfirmed::default());
}

#[test]
fn a_republished_tag_runs_the_pinned_digest_only_when_the_admin_agrees() {
    let dir = tempfile::tempdir().unwrap();
    let services = services(dir.path());
    for (answer, left) in [(false, waiting()), (true, Unconfirmed::default())] {
        let mut unconfirmed = waiting();
        let mut asked = 0;
        let mut ask = |_: &str| {
            asked += 1;
            Ok(answer)
        };
        ask_republished(
            "server",
            &services,
            &registry(OTHER),
            &accepted,
            &mut unconfirmed,
            &mut ask,
        )
        .unwrap();
        assert_eq!(asked, 1);
        assert_eq!(unconfirmed, left);
    }
}

#[test]
fn a_tag_the_registry_dropped_is_asked_about_too() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake {
        sent: Default::default(),
        answer: Box::new(|_, _, _| response(404, &[], "{}")),
    };
    let client = Client::new(Box::new(fake), None).unwrap();
    let mut unconfirmed = waiting();
    let mut asked = 0;
    let mut ask = |_: &str| {
        asked += 1;
        Ok(true)
    };
    ask_republished(
        "server",
        &services(dir.path()),
        &client,
        &accepted,
        &mut unconfirmed,
        &mut ask,
    )
    .unwrap();
    assert_eq!(asked, 1);
    assert_eq!(unconfirmed, Unconfirmed::default());
}

#[test]
fn a_recipe_built_from_an_unaccepted_commit_waits_without_a_question() {
    let dir = tempfile::tempdir().unwrap();
    let mut unconfirmed = waiting();
    let mut asked = 0;
    let mut ask = |_: &str| {
        asked += 1;
        Ok(true)
    };
    ask_republished(
        "server",
        &services(dir.path()),
        &registry(OTHER),
        &|_, _, _| Ok(Some("it was built from an unaccepted commit".into())),
        &mut unconfirmed,
        &mut ask,
    )
    .unwrap();
    assert_eq!(asked, 0);
    assert_eq!(unconfirmed, waiting());
}
