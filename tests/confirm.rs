use super::*;
use crate::container::{
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
            &mut unconfirmed,
            &mut ask,
        )
        .unwrap();
        assert_eq!(asked, 1);
        assert_eq!(unconfirmed, left);
    }
}
