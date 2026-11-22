use super::*;
use std::fs;

#[test]
fn lists_every_folder_no_service_owns() {
    let tmp = tempfile::TempDir::new().unwrap();
    let places = Places {
        projects: tmp.path().join("services"),
        data: tmp.path().join("srv"),
        owner: (0, 0),
    };
    for dir in [
        "srv/app",
        "srv/old",
        "srv/www",
        "services/app",
        "services/.app.previous",
        "services/gone",
    ] {
        fs::create_dir_all(tmp.path().join(dir)).unwrap();
    }
    let owned = BTreeSet::from(["app"]);
    let left: Vec<String> = unowned(&places, &owned)
        .unwrap()
        .iter()
        .map(|p| p.strip_prefix(tmp.path()).unwrap().display().to_string())
        .collect();
    assert_eq!(left, ["srv/old", "srv/www", "services/gone"]);
}

#[test]
fn a_machine_without_services_has_no_data_folders() {
    let tmp = tempfile::TempDir::new().unwrap();
    let places = Places {
        projects: tmp.path().join("services"),
        data: tmp.path().join("srv"),
        owner: (0, 0),
    };
    assert!(unowned(&places, &BTreeSet::new()).unwrap().is_empty());
}

#[test]
fn a_fleet_with_recipes_gets_mothership_s_workflow() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    fs::create_dir_all(dir.join("types/workstation")).unwrap();
    assert!(workflow(dir).unwrap().is_none());
    let caddy = dir.join("types/server/services/caddy");
    fs::create_dir_all(&caddy).unwrap();
    fs::write(caddy.join(super::super::DOCKERFILE), "FROM caddy:2\n").unwrap();
    assert_eq!(workflow(dir).unwrap().unwrap().detail, "missing");
    let path = dir.join(WORKFLOW_PATH);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "name: old\n").unwrap();
    assert_eq!(workflow(dir).unwrap().unwrap().detail, "outdated");
    fs::write(&path, WORKFLOW).unwrap();
    assert!(workflow(dir).unwrap().is_none());
}
