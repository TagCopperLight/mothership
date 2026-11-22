use super::*;
use crate::{
    container::registry::tests::{Fake, response},
    http::Http,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

fn digest(n: u8) -> String {
    format!("sha256:{}", format!("{n:x}").repeat(64))
}

fn registry(tags: &[&str], digests: Arc<Mutex<BTreeMap<String, String>>>) -> Client {
    let tags = serde_json::json!({ "tags": tags }).to_string();
    let fake = Fake {
        sent: Arc::default(),
        answer: Box::new(move |_, url, _| {
            if url.ends_with("/tags/list?n=1000") {
                return response(200, &[], &tags);
            }
            let tag = url.rsplit('/').next().unwrap();
            let digest = digests.lock().unwrap()[tag].clone();
            response(200, &[("Docker-Content-Digest", &digest)], "")
        }),
    };
    let http: Box<dyn Http> = Box::new(fake);
    Client::new(http, None).unwrap()
}

fn trusted(_: &Watched, _: &Pin) -> Result<()> {
    Ok(())
}

fn watched(version: &str, repo: Option<&str>) -> Watched {
    Watched {
        service: "server/db".into(),
        name: "db".into(),
        image: Image {
            image: "ghcr.io/me/db".into(),
            version: version.into(),
            digest: digest(1),
            repo: repo.map(str::to_owned),
            recipe: false,
            ports: Vec::new(),
            callers: Default::default(),
        },
        dockerfile: None,
    }
}

#[test]
fn a_public_release_waits_two_days_and_a_new_major_is_only_noticed() {
    let digests = Arc::new(Mutex::new(BTreeMap::from([
        ("16.4".to_owned(), digest(1)),
        ("16.5".to_owned(), digest(2)),
        ("16.6".to_owned(), digest(3)),
    ])));
    let client = registry(
        &[
            "16.4",
            "16.5",
            "16.6",
            "17.0",
            "16.6-alpine",
            "latest",
            "16",
        ],
        digests,
    );
    let mut state = State::default();
    let found = plan(
        &[watched("16.4", None)],
        &client,
        &mut state,
        1000,
        Round::Everything,
        &trusted,
    );
    assert!(found.updates.is_empty());
    assert_eq!(
        state.notices.values().collect::<Vec<_>>(),
        ["ghcr.io/me/db 17.0 is out, and server/db follows 16: move it by hand"]
    );
    state.seen.get_mut("ghcr.io/me/db:16.6").unwrap().since = 1000 + PUBLIC_WAIT;
    let found = plan(
        &[watched("16.4", None)],
        &client,
        &mut state,
        1000 + PUBLIC_WAIT,
        Round::Everything,
        &trusted,
    );
    assert_eq!(
        found.updates,
        [Update {
            service: "server/db".into(),
            name: "db".into(),
            from: Pin {
                version: "16.4".into(),
                digest: digest(1)
            },
            to: Pin {
                version: "16.5".into(),
                digest: digest(2)
            },
        }]
    );
}

#[test]
fn my_image_is_taken_at_once_and_a_new_digest_for_the_same_version_too() {
    let digests = Arc::new(Mutex::new(BTreeMap::from([(
        "1.2.0".to_owned(),
        digest(5),
    )])));
    let client = registry(&["1.1.0", "1.2.0"], digests.clone());
    let mut state = State::default();
    let found = plan(
        &[watched("1.2.0", Some("me/db"))],
        &client,
        &mut state,
        0,
        Round::Mine,
        &trusted,
    );
    assert_eq!(
        found.updates[0].to,
        Pin {
            version: "1.2.0".into(),
            digest: digest(5)
        }
    );
    digests.lock().unwrap().insert("1.2.0".into(), digest(1));
    let found = plan(
        &[watched("1.2.0", Some("me/db"))],
        &client,
        &mut state,
        0,
        Round::Mine,
        &trusted,
    );
    assert!(found.updates.is_empty());
}

#[test]
fn public_images_wait_for_the_hourly_round_and_mine_come_every_round() {
    let mut state = State::default();
    assert_eq!(Round::at(&state, PUBLIC_EVERY), Round::Everything);
    let digests = Arc::new(Mutex::new(BTreeMap::from([
        ("1.0".to_owned(), digest(1)),
        ("1.1".to_owned(), digest(2)),
    ])));
    let client = registry(&["1.0", "1.1"], digests);
    let items = [watched("1.0", None), watched("1.0", Some("me/db"))];
    let found = plan(
        &items,
        &client,
        &mut state,
        PUBLIC_EVERY,
        Round::Everything,
        &trusted,
    );
    assert_eq!(found.updates.len(), 1, "mine at once, the public one waits");
    assert_eq!(state.public_checked, PUBLIC_EVERY);
    assert_eq!(Round::at(&state, PUBLIC_EVERY + 300), Round::Mine);
    assert_eq!(Round::at(&state, 2 * PUBLIC_EVERY), Round::Everything);
    let public = &items[..1];
    assert!(!Round::Mine.covers(&public[0]));
    let found = plan(
        public,
        &client,
        &mut state,
        PUBLIC_EVERY + 300,
        Round::Mine,
        &trusted,
    );
    assert!(found.updates.is_empty() && found.failures.is_empty());
    assert_eq!(state.public_checked, PUBLIC_EVERY);
}

#[test]
fn a_notice_stays_between_rounds_until_its_image_goes() {
    let digests = Arc::new(Mutex::new(BTreeMap::from([("1.0".to_owned(), digest(1))])));
    let client = registry(&["1.0", "2.0"], digests);
    let mut state = State::default();
    plan(
        &[watched("1.0", None)],
        &client,
        &mut state,
        PUBLIC_EVERY,
        Round::Everything,
        &trusted,
    );
    assert_eq!(state.notices.len(), 1);
    plan(
        &[watched("1.0", None)],
        &client,
        &mut state,
        PUBLIC_EVERY + 300,
        Round::Mine,
        &trusted,
    );
    assert_eq!(state.notices.len(), 1);
    plan(
        &[],
        &client,
        &mut state,
        PUBLIC_EVERY + 600,
        Round::Mine,
        &trusted,
    );
    assert!(state.notices.is_empty());
}

#[test]
fn a_tag_the_check_refuses_leaves_the_older_ones_open() {
    let digests = Arc::new(Mutex::new(BTreeMap::from([
        ("1.0".to_owned(), digest(1)),
        ("1.1".to_owned(), digest(2)),
        ("1.2".to_owned(), digest(3)),
    ])));
    let client = registry(&["1.0", "1.1", "1.2"], digests);
    let refuse_newest = |_: &Watched, pin: &Pin| match pin.version.as_str() {
        "1.2" => anyhow::bail!("built from an old recipe"),
        _ => Ok(()),
    };
    let mut state = State::default();
    let found = plan(
        &[watched("1.0", Some("me/db"))],
        &client,
        &mut state,
        0,
        Round::Mine,
        &refuse_newest,
    );
    assert_eq!(found.updates[0].to.version, "1.1");
    assert_eq!(found.failures.len(), 1, "{:?}", found.failures);
}
