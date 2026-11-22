use super::*;
use crate::secrets::{self, Recipient};

pub const DIGEST: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";

pub fn toml(version: &str) -> String {
    format!(
        "[images.app]\nimage = \"ghcr.io/me/app\"\nversion = \"{version}\"\ndigest = \"{DIGEST}\"\n"
    )
}

#[test]
fn a_service_pins_images_with_versions_and_digests() {
    let images = parse(&toml("1.2.3")).unwrap();
    assert_eq!(
        images["app"].reference(&images["app"].pin()),
        format!("ghcr.io/me/app:1.2.3@{DIGEST}")
    );
    let err = |text: &str| format!("{:#}", parse(text).unwrap_err());
    assert!(err(&toml("latest")).contains("the fleet runs only images with versions"));
    assert!(err(&toml("1").replace(DIGEST, "sha256:12")).contains("isn't a digest"));
    assert!(err(&toml("1").replace("ghcr.io/me/app", "app:1")).contains("without a tag"));
    assert!(err("").contains("it pins no image"));
    parse(&toml("1").replace("ghcr.io/me/app", "localhost:5000/app")).unwrap();
}

fn service(dir: &Path) -> Service {
    fs::create_dir_all(dir.join(SECRETS)).unwrap();
    fs::write(dir.join(SERVICE_TOML), toml("1.0.0")).unwrap();
    fs::write(dir.join(COMPOSE), "services:\n  app: {}\n").unwrap();
    load(dir.parent().unwrap().parent().unwrap())
        .unwrap()
        .remove(0)
}

#[test]
fn rendering_decrypts_secrets_and_fills_templates() {
    let dir = tempfile::tempdir().unwrap();
    let svc = dir.path().join("services/app");
    let key = age::x25519::Identity::generate();
    let list = [Recipient {
        name: "root".into(),
        key: secrets::public(&key),
    }];
    fs::create_dir_all(svc.join(SECRETS)).unwrap();
    fs::write(
        svc.join("secrets/token.age"),
        secrets::encrypt_for(&list, b"hunter2\n").unwrap(),
    )
    .unwrap();
    fs::write(
        svc.join("app.conf.tmpl"),
        "token = {{ secret \"token\" }}\n",
    )
    .unwrap();
    let service = service(&svc);
    let mut keys = Keys::default();
    keys.push(key);
    let rendered = service.render(&keys).unwrap();
    assert_eq!(rendered.files[Path::new("app.conf")], b"token = hunter2\n");
    assert!(rendered.files.contains_key(Path::new(COMPOSE)));
    assert_eq!(rendered.secrets["token"], b"hunter2\n");
    fs::write(svc.join("bad.tmpl"), "{{ secret \"nope\" }}").unwrap();
    let err = format!("{:#}", service.render(&keys).unwrap_err());
    assert!(err.contains("the service has no secret nope"), "{err}");
}

#[test]
fn a_service_holds_no_links() {
    let dir = tempfile::tempdir().unwrap();
    let svc = dir.path().join("services/app");
    let service = service(&svc);
    std::os::unix::fs::symlink("/etc/shadow", svc.join("shadow")).unwrap();
    let err = format!("{:#}", service.render(&Keys::default()).unwrap_err());
    assert!(err.contains("is a link"), "{err}");
}

#[test]
fn a_snapshot_names_paths_inside_the_data_folder() {
    let with = |parts: &str| format!("snapshot = [{parts}]\n{}", toml("1.2.3"));
    assert_eq!(
        parse_toml(&with("\"db\", \"a/b\"")).unwrap().snapshot,
        [PathBuf::from("db"), PathBuf::from("a/b")]
    );
    for bad in ["\"../etc\"", "\"/etc\"", "\"\"", "\"./db\""] {
        let err = format!("{:#}", parse_toml(&with(bad)).unwrap_err());
        assert!(err.contains("isn't a path inside"), "{bad}: {err}");
    }
}

#[test]
fn a_versions_signer_leaves_the_snapshot_alone() {
    let before = toml("1.2.3");
    let after = format!("snapshot = [\"db\"]\n{}", toml("1.2.4"));
    let err = format!("{:#}", versions_only(&before, &after).unwrap_err());
    assert!(err.contains("what the snapshot covers"), "{err}");
}

#[test]
fn the_login_file_holds_the_token_for_ghcr() {
    let file: serde_json::Value = serde_json::from_slice(&login_file("me", "ghp_x")).unwrap();
    assert_eq!(file["auths"]["ghcr.io"]["auth"], "bWU6Z2hwX3g=");
    let client = registry::Client::new(
        Box::new(registry::tests::Fake {
            sent: Default::default(),
            answer: Box::new(|_, _, _| registry::tests::response(200, &[], "")),
        }),
        Some(&login_file("me", "ghp_x")),
    );
    assert!(client.is_ok());
}

fn recipe(dir: &Path, digest: &str, dockerfile: Option<&str>) -> Result<Vec<Service>> {
    let svc = dir.join("services/caddy");
    fs::create_dir_all(&svc).unwrap();
    let digest = match digest {
        "" => String::new(),
        digest => format!("digest = \"{digest}\"\n"),
    };
    fs::write(
        svc.join(SERVICE_TOML),
        format!("[images.caddy]\nimage = \"ghcr.io/me/caddy\"\nversion = \"2.8.4\"\n{digest}recipe = true\n"),
    )
    .unwrap();
    fs::write(svc.join(COMPOSE), "services: {caddy: {}}\n").unwrap();
    if let Some(text) = dockerfile {
        fs::write(svc.join(DOCKERFILE), text).unwrap();
    }
    load(dir)
}

const RECIPE: &str = "FROM caddy:2.8.4-builder\nLABEL org.opencontainers.image.version=\"2.8.4\"\n";

#[test]
fn a_recipe_has_its_dockerfile_and_waits_for_its_first_build() {
    let dir = tempfile::tempdir().unwrap();
    let services = recipe(dir.path(), "", Some(RECIPE)).unwrap();
    assert!(services[0].unbuilt());
    let rendered = services[0].render(&Keys::default()).unwrap();
    assert!(!rendered.files.contains_key(Path::new(DOCKERFILE)));
    let built = recipe(dir.path(), DIGEST, Some(RECIPE)).unwrap();
    assert!(!built[0].unbuilt() && built[0].images["caddy"].mine());
}

#[test]
fn a_recipe_and_its_dockerfile_come_together() {
    let err = |digest: &str, dockerfile: Option<&str>| {
        let dir = tempfile::tempdir().unwrap();
        format!(
            "{:#}",
            recipe(dir.path(), digest, dockerfile).err().unwrap()
        )
    };
    assert!(err(DIGEST, None).contains("has no Dockerfile"));
    assert!(err(DIGEST, Some("FROM caddy:2\n")).contains("names its version once"));
    let dir = tempfile::tempdir().unwrap();
    service(&dir.path().join("services/app"));
    fs::write(dir.path().join("services/app").join(DOCKERFILE), RECIPE).unwrap();
    let error = format!("{:#}", load(dir.path()).err().unwrap());
    assert!(
        error.contains("no image in it says recipe = true"),
        "{error}"
    );
    let bare = toml("1.0.0").replace(&format!("digest = \"{DIGEST}\"\n"), "");
    assert!(format!("{:#}", parse(&bare).unwrap_err()).contains("isn't a digest"));
    let both = format!("{}repo = \"me/caddy\"\nrecipe = true\n", toml("1.0.0"));
    assert!(format!("{:#}", parse(&both).unwrap_err()).contains("either from a recipe"));
}

#[test]
fn the_probe_gives_a_recipe_its_first_digest_and_never_takes_it_away() {
    let unbuilt =
        "[images.caddy]\nimage = \"ghcr.io/me/caddy\"\nversion = \"2.8.4\"\nrecipe = true\n";
    let built = unbuilt.replace("recipe", &format!("digest = \"{DIGEST}\"\nrecipe"));
    versions_only(unbuilt, &built).unwrap();
    let err = format!("{:#}", versions_only(&built, unbuilt).unwrap_err());
    assert!(err.contains("isn't a digest"), "{err}");
    let err = format!(
        "{:#}",
        versions_only(&built, &built.replace("recipe = true\n", "")).unwrap_err()
    );
    assert!(err.contains("beyond its version and digest"), "{err}");
}

#[test]
fn a_recipe_names_its_version_in_a_label() {
    assert_eq!(recipe_version(RECIPE).unwrap(), "2.8.4");
    assert_eq!(
        recipe_version("LABEL org.opencontainers.image.version=v1.2\n").unwrap(),
        "v1.2"
    );
    assert!(recipe_version("LABEL org.opencontainers.image.version=\"latest\"\n").is_err());
}

#[test]
fn the_fleet_holds_recipes_when_a_service_has_a_dockerfile() {
    let dir = tempfile::tempdir().unwrap();
    let services = dir.path().join("types/server/services/caddy");
    fs::create_dir_all(&services).unwrap();
    assert!(!holds_recipes(dir.path()).unwrap());
    fs::write(services.join(DOCKERFILE), RECIPE).unwrap();
    assert!(holds_recipes(dir.path()).unwrap());
}

#[test]
fn a_recipe_builds_a_version_the_probe_can_follow() {
    let dir = tempfile::tempdir().unwrap();
    let newer = RECIPE.replace("version=\"2.8.4\"", "version=\"2.8.5\"");
    recipe(dir.path(), DIGEST, Some(&newer)).unwrap();
    let err = |digest: &str, dockerfile: &str| {
        let dir = tempfile::tempdir().unwrap();
        format!(
            "{:#}",
            recipe(dir.path(), digest, Some(dockerfile)).err().unwrap()
        )
    };
    assert!(err("", &newer).contains("waits for 2.8.4"));
    let older = RECIPE.replace("version=\"2.8.4\"", "version=\"2.8.0\"");
    assert!(err(DIGEST, &older).contains("can't follow from 2.8.4"));
    let other = RECIPE.replace("version=\"2.8.4\"", "version=\"2.8.4-ovh\"");
    assert!(err(DIGEST, &other).contains("can't follow"));
}

fn published_service(dir: &Path, name: &str, ports: &str) {
    let svc = dir.join("services").join(name);
    fs::create_dir_all(&svc).unwrap();
    fs::write(
        svc.join(SERVICE_TOML),
        format!("{}{ports}\n", toml("1.0.0")),
    )
    .unwrap();
    fs::write(svc.join(COMPOSE), "services:\n  app: {}\n").unwrap();
}

#[test]
fn each_port_names_the_network_it_faces() {
    let images = parse(&format!(
        "{}ports.public = [\"443/tcp\", \"443/udp\"]\nports.tailscale0 = [\"53/udp\"]\n\
         ports.local = [\"5432/tcp\"]\n",
        toml("1.0.0")
    ))
    .unwrap();
    let shown: Vec<String> = images["app"]
        .ports
        .iter()
        .map(Published::to_string)
        .collect();
    assert_eq!(
        shown,
        [
            "5432/tcp on local",
            "443/tcp on public",
            "443/udp on public",
            "53/udp on tailscale0"
        ]
    );
    let err = |ports: &str| {
        format!(
            "{:#}",
            parse(&format!("{}{ports}\n", toml("1"))).unwrap_err()
        )
    };
    assert!(err("ports = [\"443/tcp\"]").contains("ports grouped by the network they face"));
    assert!(err("ports.\"wg/0\" = [\"443/tcp\"]").contains("isn't a network"));
    assert!(err("ports.averyverylongname = [\"443/tcp\"]").contains("isn't a network"));
    assert!(err("ports.lo = [\"443/tcp\"]").contains("name it local"));
    for bad in [
        "\"443\"",
        "\"0/tcp\"",
        "\"70000/tcp\"",
        "\"53/sctp\"",
        "443",
    ] {
        let err = err(&format!("ports.public = [{bad}]"));
        assert!(
            err.contains("isn't a port like") || err.contains("invalid type"),
            "{bad}: {err}"
        );
    }
}

#[test]
fn a_port_binds_to_the_addresses_of_the_network_it_faces() {
    let images = parse(&format!(
        "{}ports.public = [\"80/tcp\"]\nports.local = [\"5432/tcp\"]\n\
         ports.wg-0 = [\"53/udp\"]\n",
        toml("1.0.0")
    ))
    .unwrap();
    let ports = &images["app"].ports;
    let lookup = |name: &str| -> Result<Vec<std::net::IpAddr>> {
        assert_eq!(name, "wg-0");
        Ok(vec!["100.64.0.1".parse()?, "fd7a::1".parse()?])
    };
    let port::Bound::Ready(entries) = port::bind(ports, &lookup).unwrap() else {
        panic!("wg-0 has addresses");
    };
    let hosts: Vec<String> = entries
        .iter()
        .map(|e| {
            format!(
                "{} {}",
                e["host_ip"].as_str().unwrap_or("*"),
                e["published"]
            )
        })
        .collect();
    assert_eq!(
        hosts,
        [
            "127.0.0.1 \"5432\"",
            "::1 \"5432\"",
            "* \"80\"",
            "100.64.0.1 \"53\"",
            "fd7a::1 \"53\""
        ]
    );
    let port::Bound::Waiting(interface) = port::bind(ports, &|_| Ok(Vec::new())).unwrap() else {
        panic!("wg-0 has no address");
    };
    assert_eq!(interface, "wg-0");
}

#[test]
fn two_services_never_publish_the_same_port_on_one_network() {
    let dir = tempfile::tempdir().unwrap();
    published_service(
        dir.path(),
        "dns",
        "ports.tailscale0 = [\"53/udp\", \"53/tcp\"]",
    );
    published_service(
        dir.path(),
        "web",
        "ports.public = [\"443/tcp\", \"443/udp\"]",
    );
    published_service(dir.path(), "local", "ports.local = [\"53/udp\"]");
    let services = load(dir.path()).unwrap();
    let rules: Vec<String> = published(&services)
        .unwrap()
        .iter()
        .map(Port::bind_rule)
        .collect();
    assert_eq!(rules, ["tcp:53", "udp:53", "tcp:443", "udp:443"]);
    assert_eq!(
        interfaces(&services).into_iter().collect::<Vec<_>>(),
        ["tailscale0"]
    );
    published_service(dir.path(), "other", "ports.tailscale0 = [\"53/udp\"]");
    let err = format!("{:#}", load(dir.path()).err().unwrap());
    assert_eq!(
        err,
        "dns's images.app and other's images.app both publish 53/udp on tailscale0"
    );
    published_service(dir.path(), "other", "ports.public = [\"53/tcp\"]");
    let err = format!("{:#}", load(dir.path()).err().unwrap());
    assert_eq!(
        err,
        "dns's images.app publishes 53/tcp on tailscale0, and other's images.app on public: \
         public takes every address"
    );
}

#[test]
fn a_versions_signer_leaves_the_ports_alone() {
    let before = format!("{}ports.public = [\"443/tcp\"]\n", toml("1.2.3"));
    for after in [
        format!("{}ports.public = [\"22/tcp\"]\n", toml("1.2.4")),
        format!("{}ports.local = [\"443/tcp\"]\n", toml("1.2.4")),
    ] {
        let err = format!("{:#}", versions_only(&before, &after).unwrap_err());
        assert!(err.contains("beyond its version and digest"), "{err}");
    }
}

#[test]
fn services_docker_may_bind_only_the_declared_ports() {
    let user = crate::user::User {
        name: account::USER.into(),
        uid: 990,
        gid: 990,
        home: PathBuf::from("/var/lib/services"),
        shell: "/usr/sbin/nologin".into(),
    };
    let dir = tempfile::tempdir().unwrap();
    published_service(
        dir.path(),
        "app",
        "ports.public = [\"80/tcp\"]\nports.local = [\"443/udp\"]\nports.wg0 = [\"443/udp\"]",
    );
    let services = load(dir.path()).unwrap();
    let unit = account::unit("/usr/bin/dockerd-rootless.sh", &user, &services).unwrap();
    let rules: Vec<&str> = unit
        .lines()
        .filter(|l| l.starts_with("SocketBind"))
        .collect();
    assert_eq!(
        rules,
        [
            "SocketBindAllow=tcp:80",
            "SocketBindAllow=udp:443",
            "SocketBindDeny=1-65535"
        ]
    );
    assert!(unit.contains(
        "ExecStart=/usr/bin/dockerd-rootless.sh --default-address-pool base=172.16.0.0/12,size=24\n"
    ));
    assert!(unit.contains("User=services\n"));
    assert!(unit.contains("AmbientCapabilities=CAP_NET_BIND_SERVICE\n"));
    assert!(unit.contains("XDG_RUNTIME_DIR=/run/user/990\n"));
    assert!(unit.contains("BindsTo=user@990.service\nAfter=user@990.service "));
    assert!(unit.ends_with("WantedBy=user@990.service\n"));
    let started = account::start_unit(true);
    assert!(started.contains("ExecStart=/usr/local/bin/mothership start %I\n"));
    assert!(started.ends_with("WantedBy=sys-subsystem-net-devices-%i.device\n"));
    assert!(account::start_unit(false).ends_with("WantedBy=mothership-services.service\n"));
}
