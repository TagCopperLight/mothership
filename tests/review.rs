use super::*;
use crate::scan::Kind;

fn item(subject: &str) -> Item {
    Item {
        kind: Kind::Package,
        subject: subject.into(),
        detail: String::new(),
        secret: false,
        locked: false,
        add: None,
        no_add: None,
        revert: None,
        preview: crate::preview::Preview::None,
    }
}

#[test]
fn put_off_items_come_back_after_a_week_and_keep_their_first_date() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut later = Later {
        path: dir.path().join("later.json"),
        items: BTreeMap::new(),
    };
    let htop = item("htop");
    later.put_off(&htop, 0);
    assert!(!later.due(&htop, LATER - 1));
    assert!(later.due(&htop, LATER));
    later.put_off(&htop, LATER);
    assert!(!later.due(&htop, LATER + 1));
    assert_eq!(later.get(&htop).unwrap().first, 0);
    assert!(later.due(&item("btop"), 0));
}

#[test]
fn saving_drops_items_the_scan_no_longer_finds() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("state/later.json");
    let mut later = Later {
        path: path.clone(),
        items: BTreeMap::new(),
    };
    later.put_off(&item("htop"), 0);
    later.put_off(&item("gone"), 0);
    later.save(&[item("htop")]).unwrap();
    let saved: BTreeMap<String, Waiting> =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(saved.keys().collect::<Vec<_>>(), ["package htop"]);
}
