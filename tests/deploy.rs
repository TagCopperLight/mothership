use super::*;
use crate::{
    container::{Image, Pin, load},
    secrets::Keys,
};
use std::{cell::RefCell, collections::VecDeque, os::unix::fs::MetadataExt};

const A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[derive(Default)]
struct Fake {
    calls: RefCell<Vec<String>>,
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
        Ok(serde_json::from_str(&text)?)
    }
    fn pull(&self, _: &Project) -> Result<()> {
        self.log("pull");
        Ok(())
    }
    fn up(&self, project: &Project) -> Result<()> {
        let text = fs::read_to_string(project.dir.join(OVERRIDE))?;
        let config: serde_json::Value = serde_json::from_str(&text)?;
        let image = config["services"]["app"]["image"].as_str().unwrap();
        self.log(format!("up {}", image.split('@').next().unwrap()));
        Ok(())
    }
    fn stop(&self, _: &str) -> Result<()> {
        self.log("stop");
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
    fn snapshot(&self, _: &str, _: &Path) -> Result<()> {
        self.log("snapshot");
        match self.snapshot_fails {
            true => anyhow::bail!("the disk is full"),
            false => Ok(()),
        }
    }
    fn restore(&self, _: &str, _: &Path) -> Result<()> {
        self.log("restore");
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
        ["config", "pull", "stop", "snapshot", "up app:1.0.1"]
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
            "up app:1.0.1"
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
fn a_service_without_a_health_check_keeps_its_new_versions() {
    let mut world = World::new();
    let engine = Fake::default();
    engine.health.borrow_mut().push_back(Health::Unchecked);
    let (first, _) = world.deploy(&engine, &world.pin("1.0.0", A), None);
    engine.calls();
    engine.health.borrow_mut().push_back(Health::Unchecked);
    let newer = world.pin("1.0.1", B);
    let (second, outcome) = world.deploy(&engine, &newer, Some(&first));
    assert_eq!(outcome, Outcome::Updated);
    assert_eq!(second.running, newer.pins());
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
    assert!(engine.calls().ends_with(&["down".to_owned()]));
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
    assert_eq!(engine.calls(), ["config", "pull", "stop", "snapshot", "stop", "up app:1.0.0"]);
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
    deploy(&engine, &world.places, &wanted, Some(&first), &mut world.unconfirmed).unwrap();
    assert_eq!(fs::read_to_string(&conf).unwrap(), "version 1.0.0\n");
}
