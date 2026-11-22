use super::*;
use crate::http::Response;
use std::sync::{Arc, Mutex};

type Answer = dyn Fn(&str, &str, &[String]) -> Response + Send + Sync;
pub type Sent = Arc<Mutex<Vec<(String, String, Vec<String>)>>>;

pub struct Fake {
    pub sent: Sent,
    pub answer: Box<Answer>,
}

impl Http for Fake {
    fn send(&self, request: &Request) -> Result<Response> {
        self.sent.lock().unwrap().push((
            request.method.to_owned(),
            request.url.to_owned(),
            request.headers.clone(),
        ));
        Ok((self.answer)(request.method, request.url, &request.headers))
    }
}

pub fn response(status: u16, headers: &[(&str, &str)], body: &str) -> Response {
    Response {
        status,
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        body: body.as_bytes().to_vec(),
    }
}

const DIGEST: &str = "sha256:3333333333333333333333333333333333333333333333333333333333333333";

#[test]
fn names_find_their_registry() {
    let host = |image: &str| {
        let name = name(image);
        (name.host, name.path)
    };
    assert_eq!(
        host("postgres"),
        ("docker.io".into(), "library/postgres".into())
    );
    assert_eq!(
        host("caddy/caddy"),
        ("docker.io".into(), "caddy/caddy".into())
    );
    assert_eq!(host("ghcr.io/me/app"), ("ghcr.io".into(), "me/app".into()));
    assert_eq!(
        host("10.0.2.2:5000/app"),
        ("10.0.2.2:5000".into(), "app".into())
    );
}

#[test]
fn a_digest_comes_after_logging_in_with_the_fleets_login() {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let fake = Fake {
        sent: sent.clone(),
        answer: Box::new(|_, url, headers| {
            if url.starts_with("https://ghcr.io/token") {
                return response(200, &[], "{\"token\": \"t0k\"}");
            }
            match headers.iter().any(|h| h == "Authorization: Bearer t0k") {
                true => response(200, &[("Docker-Content-Digest", DIGEST)], ""),
                false => response(
                    401,
                    &[(
                        "WWW-Authenticate",
                        "Bearer realm=\"https://ghcr.io/token\",service=\"ghcr.io\",scope=\"repository:me/app:pull\"",
                    )],
                    "",
                ),
            }
        }),
    };
    let config = br#"{"auths": {"ghcr.io": {"auth": "bWU6c2VjcmV0"}}}"#;
    let client = Client::new(Box::new(fake), Some(config)).unwrap();
    assert_eq!(client.digest("ghcr.io/me/app", "1.2.3").unwrap(), DIGEST);
    client.digest("ghcr.io/me/app", "1.2.4").unwrap();
    let sent = sent.lock().unwrap();
    assert_eq!(sent.len(), 4, "the token is kept: {sent:?}");
    assert_eq!(
        sent[1].1,
        "https://ghcr.io/token?service=ghcr.io&scope=repository:me/app:pull"
    );
    assert_eq!(sent[1].2, ["Authorization: Basic bWU6c2VjcmV0"]);
    assert_eq!(sent[0].0, "HEAD");
}

#[test]
fn tags_follow_the_registrys_pages() {
    let fake = Fake {
        sent: Arc::default(),
        answer: Box::new(|_, url, _| match url.contains("last=") {
            false => response(
                200,
                &[("Link", "</v2/me/app/tags/list?n=1000&last=b>; rel=\"next\"")],
                "{\"tags\": [\"a\", \"b\"]}",
            ),
            true => response(200, &[], "{\"tags\": [\"c\"]}"),
        }),
    };
    let client = Client::new(Box::new(fake), None).unwrap();
    assert_eq!(client.tags("ghcr.io/me/app").unwrap(), ["a", "b", "c"]);
}

pub struct Stored {
    pub top: String,
    pub blobs: std::collections::BTreeMap<String, Vec<u8>>,
}

fn store(
    blobs: &mut std::collections::BTreeMap<String, Vec<u8>>,
    value: serde_json::Value,
) -> String {
    let bytes = value.to_string().into_bytes();
    let digest = format!("sha256:{}", crate::hash::sha256(&bytes));
    blobs.insert(digest.clone(), bytes);
    digest
}

pub fn labelled(labels: serde_json::Value) -> Stored {
    let mut blobs = std::collections::BTreeMap::new();
    let config = store(
        &mut blobs,
        serde_json::json!({ "config": { "Labels": labels } }),
    );
    let manifest = store(
        &mut blobs,
        serde_json::json!({ "config": { "digest": config } }),
    );
    let attestation = store(&mut blobs, serde_json::json!({ "config": {} }));
    let top = store(
        &mut blobs,
        serde_json::json!({ "manifests": [
            { "digest": attestation, "platform": { "os": "unknown" } },
            { "digest": manifest, "platform": { "os": "linux" } },
        ] }),
    );
    Stored { top, blobs }
}

pub fn serving(stored: &Stored, extra: Box<Answer>) -> Fake {
    let blobs = stored.blobs.clone();
    Fake {
        sent: Arc::default(),
        answer: Box::new(move |method, url, headers| {
            let last = url.rsplit('/').next().unwrap();
            let blob = url.contains("/manifests/sha256:") || url.contains("/blobs/");
            match blobs.get(last) {
                Some(bytes) if blob => response(200, &[], std::str::from_utf8(bytes).unwrap()),
                _ => extra(method, url, headers),
            }
        }),
    }
}

#[test]
fn labels_come_from_the_config_the_digests_vouch_for() {
    let stored = labelled(serde_json::json!({ "org.opencontainers.image.revision": "abc" }));
    let client = Client::new(
        Box::new(serving(&stored, Box::new(|_, _, _| response(404, &[], "")))),
        None,
    )
    .unwrap();
    let labels = client.labels("ghcr.io/me/caddy", &stored.top).unwrap();
    assert_eq!(labels["org.opencontainers.image.revision"], "abc");

    let mut forged = labelled(serde_json::json!({}));
    let top = forged.blobs.remove(&forged.top).unwrap();
    forged.blobs.insert(stored.top.clone(), top);
    let client = Client::new(
        Box::new(serving(&forged, Box::new(|_, _, _| response(404, &[], "")))),
        None,
    )
    .unwrap();
    let err = format!(
        "{:#}",
        client.labels("ghcr.io/me/caddy", &stored.top).unwrap_err()
    );
    assert!(err.contains("the registry sent"), "{err}");
}

#[test]
fn a_moved_blob_is_fetched_where_the_registry_says() {
    let stored = labelled(serde_json::json!({ "a": "b" }));
    let blobs = stored.blobs.clone();
    let fake = Fake {
        sent: Arc::default(),
        answer: Box::new(move |_, url, headers| {
            let last = url.rsplit('/').next().unwrap().to_owned();
            if let Some(name) = url.strip_prefix("https://cdn.example/") {
                assert!(headers.is_empty());
                return response(200, &[], std::str::from_utf8(&blobs[name]).unwrap());
            }
            if url.contains("/blobs/") {
                return response(
                    307,
                    &[("Location", &format!("https://cdn.example/{last}"))],
                    "",
                );
            }
            response(200, &[], std::str::from_utf8(&blobs[&last]).unwrap())
        }),
    };
    let client = Client::new(Box::new(fake), None).unwrap();
    assert_eq!(
        client.labels("ghcr.io/me/caddy", &stored.top).unwrap()["a"],
        "b"
    );
}
