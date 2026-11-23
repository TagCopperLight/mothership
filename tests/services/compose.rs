use super::*;
use serde_json::json;

fn images() -> BTreeMap<String, String> {
    BTreeMap::from([("app".to_owned(), "app:1@sha256:1".to_owned())])
}

fn err(config: Value) -> String {
    let (project, data) = (
        Path::new("/var/lib/mothership/services/app"),
        Path::new("/srv/app"),
    );
    format!(
        "{:#}",
        check(&config, "app", project, data, &images()).unwrap_err()
    )
}

#[test]
fn a_service_mounts_only_its_data_and_its_own_files() {
    let config = json!({"services": {"app": {"image": "app:1@sha256:1", "volumes": [
        {"type": "bind", "source": "/srv/app/db", "target": "/db"},
        {"type": "bind", "source": "/var/lib/mothership/services/app/app.conf", "target": "/c"},
        {"type": "tmpfs", "target": "/tmp"},
    ]}}, "secrets": {"token": {"file": "/var/lib/mothership/services/app/secrets/token"}}});
    let (project, data) = (
        Path::new("/var/lib/mothership/services/app"),
        Path::new("/srv/app"),
    );
    check(&config, "app", project, data, &images()).unwrap();

    let bind = |source: &str| {
        json!({"services": {"app": {"image": "app:1@sha256:1", "volumes": [
            {"type": "bind", "source": source, "target": "/x"}]}}})
    };
    assert_eq!(
        err(bind("/etc")),
        "app mounts /etc: a service reaches only /srv/app and its own files"
    );
    assert!(err(bind("/srv/app/../other")).contains("app mounts"));
    assert_eq!(
        err(
            json!({"services": {"app": {"image": "app:1@sha256:1", "volumes": [
            {"type": "volume", "source": "db", "target": "/db"}]}}})
        ),
        "app uses a Docker volume: keep its data in /srv/app"
    );
    assert_eq!(
        err(json!({"services": {"app": {"image": "app:1@sha256:1"}}, "volumes": {"db": {}}})),
        "compose.yaml declares Docker volumes: keep data in /srv/app"
    );
    assert_eq!(
        err(json!({"services": {"app": {"image": "app:1@sha256:1"}},
                   "secrets": {"key": {"file": "/root/key"}}})),
        "the secrets entry key reads /root/key, outside the service"
    );
}

#[test]
fn every_container_runs_its_pinned_image() {
    assert_eq!(
        err(json!({"services": {"app": {"image": "app:1@sha256:1"}, "db": {"image": "x"}}})),
        "compose.yaml runs db, which service.toml pins no image for"
    );
    assert_eq!(
        err(json!({"services": {}})),
        "service.toml pins images.app, which compose.yaml doesn't run"
    );
    assert_eq!(
        err(json!({"services": {"app": {"image": "evil:latest"}}})),
        "app doesn't run the image service.toml pins"
    );
    assert_eq!(
        err(json!({"services": {"app": {"image": "app:1@sha256:1", "build": {"context": "."}}}})),
        "app builds its image: the fleet runs only pinned images"
    );
    assert_eq!(
        err(
            json!({"services": {"app": {"image": "app:1@sha256:1", "ports": [
            {"mode": "ingress", "target": 80, "published": "80", "protocol": "tcp"}]}}})
        ),
        "app publishes ports in compose.yaml: declare them in service.toml"
    );
}

#[test]
fn a_service_stays_on_its_own_networks() {
    let app = |extra: Value| {
        let mut service = json!({"image": "app:1@sha256:1", "networks": {"back": null}});
        service
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        service
    };
    let with = |networks: Value, extra: Value| json!({"services": {"app": app(extra)}, "networks": networks});
    let (project, data) = (
        Path::new("/var/lib/mothership/services/app"),
        Path::new("/srv/app"),
    );
    let own = json!({"back": {"name": "app_back"}, "default": {"name": "app_default"}});
    check(&with(own, json!({})), "app", project, data, &images()).unwrap();

    let outside = "compose.yaml uses the network back from outside the service: another service \
                   calls it through callers in service.toml";
    assert_eq!(
        err(with(json!({"back": {"name": "other_default"}}), json!({}))),
        outside
    );
    assert_eq!(
        err(with(
            json!({"back": {"name": "app_back", "external": true}}),
            json!({})
        )),
        outside
    );
    assert!(
        err(with(
            json!({"link.a.b.c": {"name": "app_link.a.b.c"}}),
            json!({})
        ))
        .contains("names starting with link. are mothership's links")
    );
    assert_eq!(
        err(with(json!({}), json!({"network_mode": "host"}))),
        "app sets network_mode: a container stays on its service's networks"
    );
    assert!(
        err(with(json!({}), json!({"external_links": ["other-db-1"]})))
            .starts_with("app links containers outside the service")
    );
    assert_eq!(
        err(with(
            json!({}),
            json!({"volumes_from": ["container:other-db-1"]})
        )),
        "app mounts another container's volumes: a service reaches only its own files"
    );
    for (key, value) in [
        ("pid", "container:other-db-1"),
        ("ipc", "host"),
        ("uts", "host"),
    ] {
        assert_eq!(
            err(with(json!({}), json!({ key: value }))),
            format!("app shares its {key} namespace outside the service")
        );
    }
    check(
        &with(json!({}), json!({"ipc": "private"})),
        "app",
        project,
        data,
        &images(),
    )
    .unwrap();
}
