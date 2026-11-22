use super::*;
use crate::container::Image;
use crate::container::registry::tests::{Fake, response};
use std::sync::{Arc, Mutex};

#[test]
fn a_dockerfile_names_its_bases_by_version() {
    let dockerfile = "FROM --platform=$BUILDPLATFORM rust:1.91-alpine AS build\n\
        RUN cargo build\n\
        FROM build AS test\n\
        FROM ghcr.io/me/base:2 AS other\n\
        FROM scratch\n\
        COPY --from=build /app /app\n\
        from debian\n";
    assert_eq!(
        bases(dockerfile),
        [
            ("debian".to_owned(), "latest".to_owned()),
            ("ghcr.io/me/base".to_owned(), "2".to_owned()),
            ("rust".to_owned(), "1.91-alpine".to_owned()),
        ]
    );
    assert_eq!(
        bases("FROM localhost:5000/base\n"),
        [("localhost:5000/base".to_owned(), "latest".to_owned())]
    );
}

#[test]
fn a_base_with_a_new_digest_asks_the_project_to_rebuild_its_version() {
    let base = Arc::new(Mutex::new(format!("sha256:{}", "a".repeat(64))));
    let sent = Arc::new(Mutex::new(Vec::new()));
    let answer = {
        let base = base.clone();
        move |_: &str, url: &str, _: &[String]| {
            if url.contains("/contents/Dockerfile?ref=v1.2.0") {
                return response(200, &[], "FROM debian:13\n");
            }
            if url.ends_with("/dispatches") {
                return response(204, &[], "");
            }
            response(200, &[("Docker-Content-Digest", &base.lock().unwrap())], "")
        }
    };
    let fake = Fake {
        sent: sent.clone(),
        answer: Box::new(answer),
    };
    let registry = Client::new(
        Box::new(Fake {
            sent: Arc::default(),
            answer: Box::new({
                let base = base.clone();
                move |_, _, _| {
                    response(200, &[("Docker-Content-Digest", &base.lock().unwrap())], "")
                }
            }),
        }),
        None,
    )
    .unwrap();
    let github = GitHub {
        http: &fake,
        token: "t".into(),
    };
    let watched = [Watched {
        service: "server/app".into(),
        name: "app".into(),
        image: Image {
            image: "ghcr.io/me/app".into(),
            version: "1.2.0".into(),
            digest: format!("sha256:{}", "1".repeat(64)),
            repo: Some("me/app".into()),
            recipe: false,
            ports: Vec::new(),
            callers: Default::default(),
        },
        dockerfile: None,
    }];
    let mut state = State::default();
    assert!(
        ask(&github, &registry, &watched, None, &mut state)
            .asked
            .is_empty()
    );
    assert!(
        ask(&github, &registry, &watched, None, &mut state)
            .asked
            .is_empty()
    );
    *base.lock().unwrap() = format!("sha256:{}", "b".repeat(64));
    assert_eq!(
        ask(&github, &registry, &watched, None, &mut state).asked,
        ["me/app 1.2.0"]
    );
    let sent = sent.lock().unwrap();
    let (method, url, headers) = sent.last().unwrap();
    assert_eq!(
        (method.as_str(), url.as_str()),
        ("POST", "https://api.github.com/repos/me/app/dispatches")
    );
    assert!(headers.contains(&"Authorization: Bearer t".to_owned()));
}

#[test]
fn a_release_tag_may_leave_out_the_v() {
    let fake = Fake {
        sent: Arc::default(),
        answer: Box::new(|_, url, _| match url.ends_with("?ref=1.2.0") {
            true => response(200, &[], "FROM debian:13\n"),
            false => response(404, &[], "{}"),
        }),
    };
    let github = GitHub {
        http: &fake,
        token: "t".into(),
    };
    assert_eq!(
        github.dockerfile("me/app", "1.2.0").unwrap(),
        "FROM debian:13\n"
    );
    let missing = Fake {
        sent: Arc::default(),
        answer: Box::new(|_, _, _| response(404, &[], "{}")),
    };
    let github = GitHub {
        http: &missing,
        token: "t".into(),
    };
    let err = format!("{:#}", github.dockerfile("me/app", "1.2.0").unwrap_err());
    assert!(
        err.contains("no Dockerfile at a tag v1.2.0 or 1.2.0"),
        "{err}"
    );
}

#[test]
fn a_recipe_whose_base_changes_asks_the_fleet_to_rebuild_it() {
    let base = Arc::new(Mutex::new(format!("sha256:{}", "a".repeat(64))));
    let sent = Arc::new(Mutex::new(Vec::new()));
    let fake = Fake {
        sent: sent.clone(),
        answer: Box::new(|_, _, _| response(204, &[], "")),
    };
    let registry = Client::new(
        Box::new(Fake {
            sent: Arc::default(),
            answer: Box::new({
                let base = base.clone();
                move |_, _, _| {
                    response(200, &[("Docker-Content-Digest", &base.lock().unwrap())], "")
                }
            }),
        }),
        None,
    )
    .unwrap();
    let github = GitHub {
        http: &fake,
        token: "t".into(),
    };
    let watched = [Watched {
        service: "server/caddy".into(),
        name: "caddy".into(),
        image: Image {
            image: "ghcr.io/me/caddy".into(),
            version: "2.8.4".into(),
            digest: format!("sha256:{}", "1".repeat(64)),
            repo: None,
            recipe: true,
            ports: Vec::new(),
            callers: Default::default(),
        },
        dockerfile: Some("FROM caddy:2.8.4-builder\n".into()),
    }];
    let mut state = State::default();
    let fleet = Some("me/fleet");
    assert!(
        ask(&github, &registry, &watched, fleet, &mut state)
            .asked
            .is_empty()
    );
    *base.lock().unwrap() = format!("sha256:{}", "b".repeat(64));
    assert_eq!(
        ask(&github, &registry, &watched, fleet, &mut state).asked,
        ["the fleet's recipe server/caddy"]
    );
    let sent = sent.lock().unwrap();
    assert_eq!(
        sent.len(),
        1,
        "the recipe is read from the fleet, not GitHub"
    );
    assert_eq!(
        sent[0].1,
        "https://api.github.com/repos/me/fleet/dispatches"
    );
}
