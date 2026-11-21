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
        check(&config, project, data, &images()).unwrap_err()
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
    check(&config, project, data, &images()).unwrap();

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
}
