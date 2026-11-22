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
