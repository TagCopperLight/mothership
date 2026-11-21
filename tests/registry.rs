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
