use super::*;
use crate::services::{
    self, load,
    registry::tests::{Fake, response},
    tests::{DIGEST, toml},
};
use crate::trust::verify::tests::{EVERYTHING, NEW, OLD, Repo, pinned};
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

#[test]
fn pins_a_versions_signer_sets_wait_until_a_full_signer_changes_them() {
    let mut unconfirmed = Unconfirmed::default();
    let pin = |version: &str| services::Pin {
        version: version.into(),
        digest: NEW.into(),
    };
    unconfirmed.note("server/db", vec![("db".into(), pin("16.5.0"))], true);
    assert!(unconfirmed.waits("server/db", "db", &pin("16.5.0")));
    assert!(!unconfirmed.waits("server/db", "db", &pin("16.4.0")));
    unconfirmed.note("server/db", vec![("db".into(), pin("16.6.0"))], false);
    assert_eq!(unconfirmed, Unconfirmed::default());
}

#[test]
fn a_new_machine_still_checks_the_pins_the_probe_set_before_its_anchor() {
    let repo = Repo::new();
    let (laptop, laptop_file) = repo.key("laptop");
    let (probe, probe_file) = repo.key("probe");
    repo.machine("laptop", &laptop, EVERYTHING);
    repo.machine("box", &probe, "versions");
    let service = "types/server/services/db/service.toml";
    repo.write(service, &pinned("16.4.3", OLD));
    repo.commit("Fleet", None);
    repo.write(service, &pinned("16.5.0", NEW));
    repo.commit("Probe", Some(&probe_file));
    repo.write("types/server/system.toml", "packages = []\n");
    let tip = repo.commit("Unrelated", Some(&laptop_file));
    let git = Git::at(repo.work.join(".git"));
    let pin = |version: &str, digest: &str| services::Pin {
        version: version.into(),
        digest: digest.into(),
    };
    let mut unconfirmed = Unconfirmed::default();
    inherited(&git, &tip, &mut unconfirmed).unwrap();
    assert!(unconfirmed.waits("server/db", "db", &pin("16.5.0", NEW)));

    repo.write(service, &pinned("16.6.0", NEW));
    let full = repo.commit("By hand", Some(&laptop_file));
    let mut unconfirmed = Unconfirmed::default();
    inherited(&git, &full, &mut unconfirmed).unwrap();
    assert_eq!(unconfirmed, Unconfirmed::default());
}
