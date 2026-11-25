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

#[test]
fn a_container_keeps_docker_s_default_rights_and_may_get_a_device() {
    let app = |extra: Value| {
        let mut app = json!({"image": "app:1@sha256:1"});
        app.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        json!({"services": {"app": app}})
    };
    let ok = |config: Value| {
        let (project, data) = (
            Path::new("/var/lib/mothership/services/app"),
            Path::new("/srv/app"),
        );
        check(&config, "app", project, data, &images()).unwrap();
    };
    ok(app(
        json!({"devices": [{"source": "/dev/dri", "target": "/dev/dri"}]}),
    ));
    ok(app(
        json!({"security_opt": ["no-new-privileges:true"], "privileged": false}),
    ));
    assert_eq!(
        err(app(json!({"privileged": true}))),
        "app runs privileged: a container keeps Docker's default rights"
    );
    assert_eq!(
        err(app(json!({"cap_add": ["NET_ADMIN"]}))),
        "app adds capabilities: a container keeps Docker's default rights"
    );
    assert_eq!(
        err(app(json!({"security_opt": ["seccomp:unconfined"]}))),
        "app sets security_opt seccomp:unconfined: only no-new-privileges is allowed"
    );
}

#[test]
fn a_key_mothership_doesn_t_know_is_refused() {
    let app = |extra: Value| {
        let mut app = json!({"image": "app:1@sha256:1"});
        app.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        json!({"services": {"app": app}})
    };
    for key in [
        "use_api_socket",
        "post_start",
        "pre_stop",
        "pre_start",
        "label_file",
    ] {
        assert_eq!(
            err(app(json!({ key: true }))),
            format!("app sets {key}, which mothership doesn't accept")
        );
    }
    let mut top = app(json!({}));
    top["models"] = json!({});
    assert_eq!(
        err(top),
        "compose.yaml sets models, which mothership doesn't accept"
    );
    assert_eq!(
        err(json!({"services": {"app": {"image": "app:1@sha256:1"}},
                   "secrets": {"key": {"external": true}}})),
        "the secrets entry key sets external, which mothership doesn't accept"
    );
}

#[test]
fn an_env_file_stays_in_the_service_s_folder() {
    let project = Path::new("/var/lib/mothership/services/app");
    let env = |file: &str| format!("services:\n  app:\n    env_file: {file}\n");
    for file in [
        "app.env",
        "./app.env",
        "[app.env, {path: conf/db.env, required: false}]",
    ] {
        check_written(&env(file), project).unwrap();
    }
    let refused = |file: &str| format!("{:#}", check_written(&env(file), project).unwrap_err());
    assert_eq!(
        refused("../other/secrets/token"),
        "app reads /var/lib/mothership/services/app/../other/secrets/token, outside the service"
    );
    assert_eq!(
        refused("[{path: /srv/app/app.env}]"),
        "app reads /srv/app/app.env, outside the service"
    );
    assert_eq!(
        refused("${HOME}/.docker/config.json"),
        "app reads ${HOME}/.docker/config.json: name an env_file without variables"
    );
    assert!(refused("~/x").starts_with("app reads ~/x"));
}

#[test]
fn compose_yaml_is_checked_before_docker_reads_the_files_it_names() {
    let refused = |text: &str| format!("{:#}", check_written(text, Path::new("/p")).unwrap_err());
    assert_eq!(
        refused("include: [../other/compose.yaml]\nservices: {app: {}}\n"),
        "compose.yaml sets include, which mothership doesn't accept"
    );
    assert_eq!(
        refused("services:\n  app:\n    extends: {file: ../other/compose.yaml, service: x}\n"),
        "app sets extends, which mothership doesn't accept"
    );
    assert_eq!(
        refused("x-base: &base\n  use_api_socket: true\nservices:\n  app:\n    <<: *base\n"),
        "app sets use_api_socket, which mothership doesn't accept"
    );
    assert!(
        check_written(
            "services:\n  app:\n    !!str use_api_socket: true\n",
            Path::new("/p")
        )
        .is_err()
    );
    assert!(check_written("services: {app: {}}\n---\ninclude: [x]\n", Path::new("/p")).is_err());
    for text in [
        include_str!("../../e2e/vm/services/types/server/services/app/compose.yaml"),
        include_str!("../../e2e/vm/services/types/server/services/extra/compose.yaml"),
        include_str!("../../e2e/vm/services/types/server/services/lonely/compose.yaml"),
        "x-base: &base\n  restart: always\nservices:\n  app:\n    <<: *base\n    user: \"1000\"\n",
    ] {
        check_written(text, Path::new("/p")).unwrap();
    }
}
