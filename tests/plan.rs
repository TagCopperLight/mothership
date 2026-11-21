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

fn watched(version: &str, repo: Option<&str>) -> Watched {
    Watched {
        service: "server/db".into(),
        name: "db".into(),
        image: Image {
            image: "ghcr.io/me/db".into(),
            version: version.into(),
            digest: digest(1),
            repo: repo.map(str::to_owned),
        },
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
    let found = plan(&[watched("16.4", None)], &client, &mut state, 1000);
    assert!(found.updates.is_empty());
    assert_eq!(
        found.notices,
        ["ghcr.io/me/db 17.0 is out, and server/db follows 16: move it by hand"]
    );
    state.seen.get_mut("ghcr.io/me/db:16.6").unwrap().since = 1000 + PUBLIC_WAIT;
    let found = plan(
        &[watched("16.4", None)],
        &client,
        &mut state,
        1000 + PUBLIC_WAIT,
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
    let found = plan(&[watched("1.2.0", Some("me/db"))], &client, &mut state, 0);
    assert_eq!(
        found.updates[0].to,
        Pin {
            version: "1.2.0".into(),
            digest: digest(5)
        }
    );
    digests.lock().unwrap().insert("1.2.0".into(), digest(1));
    let found = plan(&[watched("1.2.0", Some("me/db"))], &client, &mut state, 0);
    assert!(found.updates.is_empty());
}
