use super::*;
use crate::scan::{Change, Kind};
use std::fs;

fn item(subject: &str) -> Item {
    Item::new(Kind::Package, Change::Added, subject, "installed")
}

fn fleet(dir: &Path) -> Fleet {
    let type_dir = dir.join("types/workstation");
    fs::create_dir_all(type_dir.join("etc")).unwrap();
    fs::write(type_dir.join(crate::fleet::SYSTEM_TOML), "").unwrap();
    Fleet::new(dir.to_path_buf(), "workstation", &[], &Default::default()).unwrap()
}

#[test]
fn put_off_items_come_back_after_a_week_and_keep_their_first_date() {
    let dir = tempfile::TempDir::new().unwrap();
    let fleet = fleet(dir.path());
    let mut later = Later::at(&dir.path().join("later.json")).unwrap();
    let htop = item("htop");
    later.put_off(&fleet, &htop, 0);
    assert!(!later.due(&fleet, &htop, LATER - 1));
    assert!(later.due(&fleet, &htop, LATER));
    later.put_off(&fleet, &htop, LATER);
    assert!(!later.due(&fleet, &htop, LATER + 1));
    assert_eq!(later.get(&fleet, &htop).unwrap().first, 0);
    assert!(later.due(&fleet, &item("btop"), 0));
}

#[test]
fn days_left_round_up() {
    let dir = tempfile::TempDir::new().unwrap();
    let fleet = fleet(dir.path());
    let mut later = Later::at(&dir.path().join("later.json")).unwrap();
    let htop = item("htop");
    later.put_off(&fleet, &htop, 0);
    assert_eq!(later.days_left(&fleet, &htop, 0), Some(7));
    assert_eq!(later.days_left(&fleet, &htop, 6 * DAY + 1), Some(1));
    assert_eq!(later.days_left(&fleet, &htop, LATER), None);
}

#[test]
fn saving_drops_items_the_scan_no_longer_finds() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("state/later.json");
    let fleet = fleet(dir.path());
    let mut later = Later::at(&path).unwrap();
    later.put_off(&fleet, &item("htop"), 0);
    later.put_off(&fleet, &item("gone"), 0);
    later.save(&fleet, &[&item("htop")]).unwrap();
    let saved: BTreeMap<String, Waiting> =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(saved.keys().collect::<Vec<_>>(), ["package htop"]);
}
