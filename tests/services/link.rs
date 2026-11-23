use super::*;
use crate::services::{COMPOSE, SERVICE_TOML, load, tests::toml, versions_only};
use std::{fs, path::Path};

fn service(dir: &Path, name: &str, callers: &str) {
    let svc = dir.join("services").join(name);
    fs::create_dir_all(&svc).unwrap();
    let text = format!("{}{callers}\n", toml("1.0.0"));
    fs::write(svc.join(SERVICE_TOML), text).unwrap();
    fs::write(svc.join(COMPOSE), "services:\n  app: {}\n").unwrap();
}

fn err(dir: &Path) -> String {
    format!("{:#}", load(dir).err().unwrap())
}

#[test]
fn a_link_joins_the_reached_container_and_only_the_caller_container() {
    let dir = tempfile::tempdir().unwrap();
    service(
        dir.path(),
        "proxy",
        "callers = [\"gateway/app\", \"monitor/app\"]",
    );
    service(dir.path(), "gateway", "");
    let web = toml("1.0.0").replace("[images.app]", "[images.web]");
    let gateway = dir.path().join("services/gateway");
    let text = fs::read_to_string(gateway.join(SERVICE_TOML)).unwrap();
    fs::write(gateway.join(SERVICE_TOML), format!("{text}\n{web}")).unwrap();
    service(dir.path(), "monitor", "");
    service(dir.path(), "alone", "");
    let services = load(dir.path()).unwrap();
    let links = |name: &str| -> Vec<String> {
        let service = services.iter().find(|s| s.name == name).unwrap();
        service.links.iter().map(Link::network).collect()
    };
    assert_eq!(
        links("proxy"),
        ["link.proxy.app.gateway.app", "link.proxy.app.monitor.app"]
    );
    assert_eq!(links("gateway"), ["link.proxy.app.gateway.app"]);
    assert_eq!(links("monitor"), ["link.proxy.app.monitor.app"]);
    assert!(links("alone").is_empty());
    assert_eq!(
        networks(&services).into_iter().collect::<Vec<_>>(),
        ["link.proxy.app.gateway.app", "link.proxy.app.monitor.app"]
    );

    let config = json!({"services": {
        "app": {"networks": {"default": null}},
        "web": {"networks": {"default": null}}}});
    let proxy = services.iter().find(|s| s.name == "proxy").unwrap();
    let (joined, declared) = join(proxy, &config).unwrap();
    assert_eq!(
        Value::Object(joined),
        json!({"app": {"default": null,
            "link.proxy.app.gateway.app": {"aliases": ["app.proxy"]},
            "link.proxy.app.monitor.app": {"aliases": ["app.proxy"]}}})
    );
    assert_eq!(
        declared["link.proxy.app.gateway.app"],
        json!({"name": "link.proxy.app.gateway.app", "external": true})
    );
    let gateway = services.iter().find(|s| s.name == "gateway").unwrap();
    let (joined, _) = join(gateway, &config).unwrap();
    assert_eq!(
        Value::Object(joined),
        json!({"app": {"default": null, "link.proxy.app.gateway.app": null}})
    );
    let alone = services.iter().find(|s| s.name == "alone").unwrap();
    assert!(join(alone, &config).unwrap().0.is_empty());
}

#[test]
fn a_link_names_a_container_of_another_service_of_the_type() {
    let dir = tempfile::tempdir().unwrap();
    service(dir.path(), "proxy", "callers = [\"gateway/app\"]");
    assert_eq!(
        err(dir.path()),
        "proxy's images.app lets gateway call it, and the type has no service gateway"
    );
    service(dir.path(), "gateway", "");
    service(dir.path(), "proxy", "callers = [\"gateway/web\"]");
    assert_eq!(
        err(dir.path()),
        "proxy's images.app lets gateway/web call it, and gateway runs no web"
    );
    service(dir.path(), "proxy", "callers = [\"gateway\"]");
    assert!(
        err(dir.path())
            .contains("the caller gateway names no container: write gateway/<container>")
    );
    service(dir.path(), "proxy", "callers = [\"proxy/app\"]");
    assert!(err(dir.path()).contains("lets its own service call it"));
    service(dir.path(), "proxy", "callers = [\"Gate_way/app\"]");
    assert!(err(dir.path()).contains("Gate_way isn't a hostname"));
}

#[test]
fn a_versions_signer_leaves_the_links_alone() {
    let before = format!("{}callers = [\"gateway/app\"]\n", toml("1.2.3"));
    for after in [
        toml("1.2.4"),
        format!(
            "{}callers = [\"gateway/app\", \"other/app\"]\n",
            toml("1.2.4")
        ),
    ] {
        let err = format!("{:#}", versions_only(&before, &after).unwrap_err());
        assert!(err.contains("beyond its version and digest"), "{err}");
    }
    versions_only(&before, &before.replace("1.2.3", "1.2.4")).unwrap();
}
