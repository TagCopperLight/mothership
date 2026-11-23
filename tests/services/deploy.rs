use super::*;
use crate::{
    secrets::Keys,
    services::{Image, Pin, load},
};
use std::{cell::RefCell, collections::VecDeque, os::unix::fs::MetadataExt};

const A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn with_parts(verb: &str, parts: &[PathBuf]) -> String {
    let parts: Vec<String> = parts.iter().map(|p| p.display().to_string()).collect();
    [&[verb.to_owned()][..], &parts].concat().join(" ")
}

#[derive(Default)]
struct Fake {
    calls: RefCell<Vec<String>>,
    envs: RefCell<Vec<Vec<(String, String)>>>,
    health: RefCell<VecDeque<Health>>,
    snapshot_fails: bool,
    registry: Option<String>,
}

impl Fake {
    fn log(&self, call: impl Into<String>) {
        self.calls.borrow_mut().push(call.into());
    }

    fn calls(&self) -> Vec<String> {
        self.calls.take()
    }
}

impl Engine for Fake {
    fn config(&self, project: &Project) -> Result<serde_json::Value> {
        let text = fs::read_to_string(project.dir.join(OVERRIDE))?;
        self.log("config");
        let mut config: serde_json::Value = serde_json::from_str(&text)?;
        for service in config["services"].as_object_mut().unwrap().values_mut() {
            service["networks"] = serde_json::json!({ "default": null });
        }
        Ok(config)
    }
    fn pull(&self, _: &Project) -> Result<()> {
        self.log("pull");
        Ok(())
    }
    fn up(&self, project: &Project) -> Result<()> {
        let text = fs::read_to_string(project.dir.join(OVERRIDE))?;
        let config: serde_json::Value = serde_json::from_str(&text)?;
        let image = config["services"]["app"]["image"].as_str().unwrap();
        self.envs.borrow_mut().push(project.env.clone());
        self.log(format!("up {}", image.split('@').next().unwrap()));
        Ok(())
    }
    fn stop(&self, _: &str) -> Result<()> {
        self.log("stop");
        Ok(())
    }
    fn start(&self, _: &str) -> Result<()> {
        self.log("start");
        Ok(())
    }
    fn down(&self, _: &str) -> Result<()> {
        self.log("down");
        Ok(())
    }
    fn health(&self, _: &str) -> Result<Health> {
        Ok(self
            .health
            .borrow_mut()
            .pop_front()
            .unwrap_or(Health::Healthy))
    }
    fn snapshot(&self, _: &str, _: &Path, parts: &[PathBuf]) -> Result<()> {
        self.log(with_parts("snapshot", parts));
        match self.snapshot_fails {
            true => anyhow::bail!("the disk is full"),
            false => Ok(()),
        }
    }
    fn restore(&self, _: &str, _: &Path, parts: &[PathBuf]) -> Result<()> {
        self.log(with_parts("restore", parts));
        Ok(())
    }
    fn drop_snapshot(&self, _: &str) -> Result<()> {
        self.log("drop snapshot");
        Ok(())
    }
    fn link(&self, network: &str) -> Result<()> {
        self.log(format!("link {network}"));
        Ok(())
    }
    fn confirm(&self, _: &Image, pin: &Pin) -> Result<Option<String>> {
        self.log("confirm");
        Ok(match &self.registry {
            Some(digest) if *digest != pin.digest => Some(format!("the registry gives {digest}")),
            _ => None,
        })
    }
}

struct World {
    _dir: tempfile::TempDir,
    fleet: PathBuf,
    places: Places,
    unconfirmed: Unconfirmed,
}

impl World {
    fn new() -> World {
        let dir = tempfile::tempdir().unwrap();
        let fleet = dir.path().join("type");
        let meta = fs::metadata(dir.path()).unwrap();
        let places = Places {
            projects: dir.path().join("projects"),
            data: dir.path().join("srv"),
            owner: (meta.uid(), meta.gid()),
            lookup: |_| Ok(Vec::new()),
        };
        World {
            _dir: dir,
            fleet,
            places,
            unconfirmed: Unconfirmed::default(),
        }
    }

    fn pin(&self, version: &str, digest: &str) -> Service {
        let dir = self.fleet.join("services/app");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("service.toml"),
            format!(
                "[images.app]\nimage = \"app\"\nversion = \"{version}\"\ndigest = \"{digest}\"\n"
            ),
        )
        .unwrap();
        fs::write(dir.join(COMPOSE), "services: {app: {}}\n").unwrap();
        fs::write(dir.join("app.conf"), format!("version {version}\n")).unwrap();
        load(&self.fleet).unwrap().remove(0)
    }

    fn deploy(
        &mut self,
        engine: &Fake,
        service: &Service,
        before: Option<&Deployed>,
    ) -> (Deployed, Outcome) {
        let rendered = service.render(&Keys::default()).unwrap();
        let wanted = Wanted {
            service,
            key: "server/app".into(),
            fleet: fingerprint(service, &rendered, None),
            rendered: &rendered,
            overwrite: Vec::new(),
        };
        deploy(engine, &self.places, &wanted, before, &mut self.unconfirmed).unwrap()
    }
}

#[test]
fn a_new_service_gets_its_files_and_its_data_folder_then_starts() {
    let mut world = World::new();
    let engine = Fake::default();
    let service = world.pin("1.0.0", A);
    let (entry, outcome) = world.deploy(&engine, &service, None);
    assert_eq!(outcome, Outcome::Started);
    assert_eq!(engine.calls(), ["config", "pull", "up app:1.0.0"]);
    let project = world.places.project("app");
    assert_eq!(
        fs::read_to_string(project.join("app.conf")).unwrap(),
        "version 1.0.0\n"
    );
    assert_eq!(fs::metadata(&project).unwrap().mode() & 0o777, 0o750);
    assert_eq!(
        fs::metadata(world.places.data("app")).unwrap().mode() & 0o777,
        0o700
    );
    assert_eq!(entry.running, service.pins());
    assert_eq!(entry.files.len(), 2);
}

#[test]
fn an_update_snapshots_the_data_and_a_failed_one_rolls_back_for_good() {
    let mut world = World::new();
    let engine = Fake::default();
    let (first, _) = world.deploy(&engine, &world.pin("1.0.0", A), None);
    engine.calls();

    let (second, outcome) = world.deploy(&engine, &world.pin("1.0.1", B), Some(&first));
    assert_eq!(outcome, Outcome::Updated);
    assert_eq!(
        engine.calls(),
        [
            "config",
            "pull",
            "stop",
            "snapshot",
            "up app:1.0.1",
            "drop snapshot"
        ]
    );

    engine
        .health
        .borrow_mut()
        .push_back(Health::Unhealthy("app reports unhealthy".into()));
    let broken = world.pin("1.0.2", A);
    let (third, outcome) = world.deploy(&engine, &broken, Some(&second));
    assert_eq!(outcome, Outcome::RolledBack("app reports unhealthy".into()));
    assert_eq!(
        engine.calls(),
        [
            "config",
            "pull",
            "stop",
            "snapshot",
            "up app:1.0.2",
            "stop",
            "restore",
            "up app:1.0.1",
            "drop snapshot"
        ]
    );
    assert_eq!(third.running, second.running);
    assert!(
        third
            .failed
            .contains(&("app".into(), broken.images["app"].pin()))
    );

    let (fourth, outcome) = world.deploy(&engine, &broken, Some(&third));
    assert_eq!(outcome, Outcome::Blocked("images.app 1.0.2".into()));
    assert!(engine.calls().is_empty());
    assert_eq!(fourth.running, second.running);
}

#[test]
fn a_rollback_hands_the_old_version_its_own_secrets() {
    let mut world = World::new();
    let engine = Fake::default();
    let (first, _) = world.deploy(&engine, &world.pin("1.0.0", A), None);
    let secrets = world.places.project("app").join(SECRETS);
    fs::create_dir_all(&secrets).unwrap();
    fs::write(secrets.join("db-password"), "old\n").unwrap();
    engine.envs.take();

    engine
        .health
        .borrow_mut()
        .push_back(Health::Unhealthy("app reports unhealthy".into()));
    let (_, outcome) = world.deploy(&engine, &world.pin("1.0.1", B), Some(&first));
    assert_eq!(outcome, Outcome::RolledBack("app reports unhealthy".into()));
    let old = vec![("SECRET_DB_PASSWORD".to_owned(), "old".to_owned())];
    assert_eq!(engine.envs.take(), [Vec::new(), old]);
}

#[test]
fn a_service_snapshots_only_the_parts_it_names() {
    let mut world = World::new();
    let engine = Fake::default();
    let (first, _) = world.deploy(&engine, &world.pin("1.0.0", A), None);
    engine.calls();
    let toml = world.fleet.join("services/app/service.toml");
    let text = fs::read_to_string(&toml).unwrap();
    fs::write(&toml, format!("snapshot = [\"db\", \"conf/keys\"]\n{text}")).unwrap();
    let mut service = load(&world.fleet).unwrap().remove(0);
    service.images.get_mut("app").unwrap().version = "1.0.1".into();
    engine
        .health
        .borrow_mut()
        .push_back(Health::Unhealthy("app reports unhealthy".into()));
    world.deploy(&engine, &service, Some(&first));
    let calls = engine.calls();
    assert!(
        calls.contains(&"snapshot db conf/keys".to_owned()),
        "{calls:?}"
    );
    assert!(
        calls.contains(&"restore db conf/keys".to_owned()),
        "{calls:?}"
    );
}

#[test]
fn an_edited_file_stays_and_a_dropped_service_leaves_its_data() {
    let mut world = World::new();
    let engine = Fake::default();
    let (first, _) = world.deploy(&engine, &world.pin("1.0.0", A), None);
    let conf = world.places.project("app").join("app.conf");
    fs::write(&conf, "mine\n").unwrap();
    assert_eq!(
        edited(&world.places, "app", &first).unwrap(),
        [PathBuf::from("app.conf")]
    );
    let (second, _) = world.deploy(&engine, &world.pin("1.0.1", B), Some(&first));
    assert_eq!(fs::read_to_string(&conf).unwrap(), "mine\n");

    fs::remove_file(&conf).unwrap();
    remove(&engine, &world.places, "app", &second).unwrap();
    assert!(
        engine
            .calls()
            .ends_with(&["down".to_owned(), "drop snapshot".to_owned()])
    );
    assert!(!world.places.project("app").exists());
    assert!(world.places.data("app").is_dir());
}

#[test]
fn a_pin_the_probe_set_waits_for_the_registry_to_agree() {
    let mut world = World::new();
    let mut engine = Fake {
        registry: Some(A.into()),
        ..Fake::default()
    };
    let (first, _) = world.deploy(&engine, &world.pin("1.0.0", A), None);
    engine.calls();
    let lying = world.pin("1.0.1", B);
    world.unconfirmed.note(
        "server/app",
        vec![("app".into(), lying.images["app"].pin())],
        true,
    );
    let (second, outcome) = world.deploy(&engine, &lying, Some(&first));
    assert_eq!(outcome, Outcome::Refused(format!("the registry gives {A}")));
    assert_eq!(engine.calls(), ["confirm"]);
    assert_eq!(second.running, first.running);

    engine.registry = Some(B.into());
    let (_, outcome) = world.deploy(&engine, &lying, Some(&second));
    assert_eq!(outcome, Outcome::Updated);
    assert!(
        !world
            .unconfirmed
            .waits("server/app", "app", &lying.images["app"].pin())
    );
}

#[test]
fn a_snapshot_that_fails_puts_the_old_version_back_without_touching_the_data() {
    let mut world = World::new();
    let engine = Fake {
        snapshot_fails: true,
        ..Fake::default()
    };
    let (first, _) = world.deploy(&engine, &world.pin("1.0.0", A), None);
    engine.calls();
    let (second, outcome) = world.deploy(&engine, &world.pin("1.0.1", B), Some(&first));
    assert_eq!(outcome, Outcome::RolledBack("the disk is full".into()));
    assert_eq!(
        engine.calls(),
        [
            "config",
            "pull",
            "stop",
            "snapshot",
            "stop",
            "up app:1.0.0",
            "drop snapshot"
        ]
    );
    assert_eq!(second.running, first.running);
    assert_eq!(second.files, first.files);
}

#[test]
fn overwriting_an_edited_file_writes_the_fleets_version() {
    let mut world = World::new();
    let engine = Fake::default();
    let service = world.pin("1.0.0", A);
    let (first, _) = world.deploy(&engine, &service, None);
    let conf = world.places.project("app").join("app.conf");
    fs::write(&conf, "mine\n").unwrap();
    let rendered = service.render(&Keys::default()).unwrap();
    let wanted = Wanted {
        service: &service,
        key: "server/app".into(),
        fleet: fingerprint(&service, &rendered, None),
        rendered: &rendered,
        overwrite: vec![PathBuf::from("app.conf")],
    };
    deploy(
        &engine,
        &world.places,
        &wanted,
        Some(&first),
        &mut world.unconfirmed,
    )
    .unwrap();
    assert_eq!(fs::read_to_string(&conf).unwrap(), "version 1.0.0\n");
}

#[test]
fn a_service_facing_an_interface_follows_its_addresses_and_waits_without_one() {
    let mut world = World::new();
    let engine = Fake::default();
    world.pin("1.0.0", A);
    let toml = world.fleet.join("services/app/service.toml");
    let text = fs::read_to_string(&toml).unwrap();
    fs::write(&toml, format!("{text}ports.wg0 = [\"53/udp\"]\n")).unwrap();
    let service = load(&world.fleet).unwrap().remove(0);
    let rendered = service.render(&Keys::default()).unwrap();
    let wanted = Wanted {
        service: &service,
        key: "server/app".into(),
        fleet: fingerprint(&service, &rendered, None),
        rendered: &rendered,
        overwrite: Vec::new(),
    };
    let err = deploy(
        &engine,
        &world.places,
        &wanted,
        None,
        &mut world.unconfirmed,
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("publishes on wg0, which has no address yet"));
    engine.calls();

    world.places.lookup = |_| Ok(vec!["100.64.0.1".parse()?]);
    let (entry, _) = world.deploy(&engine, &service, None);
    let host = |world: &World| {
        let text = fs::read_to_string(world.places.project("app").join(OVERRIDE)).unwrap();
        let config: serde_json::Value = serde_json::from_str(&text).unwrap();
        config["services"]["app"]["ports"][0]["host_ip"].clone()
    };
    assert_eq!(host(&world), "100.64.0.1");
    let mut state = crate::services::state::State::default();
    state.services.insert("app".into(), entry);
    let start = |world: &World, facing: Option<&str>| {
        engine.calls();
        let failed =
            crate::services::start::services(&engine, &world.places, &state, facing, &|_| true);
        assert!(failed.is_empty(), "{failed:?}");
        engine.calls()
    };
    assert_eq!(start(&world, None), ["start"]);
    assert!(start(&world, Some("eth0")).is_empty());

    world.places.lookup = |_| Ok(vec!["100.64.0.2".parse()?]);
    assert_eq!(start(&world, Some("wg0")), ["up app:1.0.0"]);
    assert_eq!(host(&world), "100.64.0.2");

    world.places.lookup = |_| Ok(Vec::new());
    assert!(start(&world, None).is_empty());
    assert_eq!(host(&world), "100.64.0.2");
}

#[test]
fn a_link_puts_the_reached_container_and_every_caller_container_on_one_network() {
    let mut world = World::new();
    let engine = Fake::default();
    world.pin("1.0.0", A);
    let app = world.fleet.join("services/app");
    let text = fs::read_to_string(app.join("service.toml")).unwrap();
    fs::write(
        app.join("service.toml"),
        format!("{text}callers = [\"gate\"]\n"),
    )
    .unwrap();
    let gate = world.fleet.join("services/gate");
    fs::create_dir_all(&gate).unwrap();
    fs::write(
        gate.join("service.toml"),
        text.replace("[images.app]", "[images.web]"),
    )
    .unwrap();
    fs::write(gate.join(COMPOSE), "services: {web: {}}\n").unwrap();
    let services = load(&world.fleet).unwrap();
    let override_of = |world: &World, name: &str| -> serde_json::Value {
        let text = fs::read_to_string(world.places.project(name).join(OVERRIDE)).unwrap();
        serde_json::from_str(&text).unwrap()
    };
    let unlinked = world.pin("1.0.0", A);
    let (before, _) = world.deploy(&engine, &unlinked, None);
    assert!(override_of(&world, "app").get("networks").is_none());
    engine.calls();

    let network = "link.app.app.gate";
    let (entry, outcome) = world.deploy(&engine, &services[0], Some(&before));
    assert_eq!(outcome, Outcome::Updated);
    assert_ne!(
        entry.fleet, before.fleet,
        "a new link redeploys the service"
    );
    let config = override_of(&world, "app");
    assert_eq!(config["networks"][network]["external"], true);
    assert_eq!(
        config["services"]["app"]["networks"][network]["aliases"],
        serde_json::json!(["app.app"])
    );
    let calls = engine.calls();
    let link = calls
        .iter()
        .position(|c| *c == format!("link {network}"))
        .unwrap();
    let up = calls.iter().position(|c| c.starts_with("up")).unwrap();
    assert!(
        link < up,
        "the network exists before the service starts: {calls:?}"
    );

    let mut state = crate::services::state::State::default();
    state.services.insert("app".into(), entry);
    let failed = crate::services::start::services(&engine, &world.places, &state, None, &|_| true);
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(engine.calls(), [format!("link {network}"), "start".into()]);
}
