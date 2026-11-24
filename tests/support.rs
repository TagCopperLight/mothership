use crate::{
    fleet::{Fleet, MANIFEST, Repo, SYSTEM_TOML},
    sync::item::{Item, Section},
    system::user::{self, User},
};
use std::{fs, path::Path};

pub fn current_user() -> User {
    user::by_uid(unsafe { libc::geteuid() }).unwrap()
}

pub fn in_section(mut item: Item, section: Section) -> Item {
    item.section = section;
    item
}

pub fn name_recovery(dir: &Path) -> age::x25519::Identity {
    let key = age::x25519::Identity::generate();
    let signing = crate::trust::sign::public(&crate::trust::sign::recovery_key(&key)).unwrap();
    let text = format!(
        "[recovery]\nage = \"{}\"\nsigning = \"{signing}\"\n",
        key.to_public()
    );
    fs::write(dir.join(MANIFEST), text).unwrap();
    key
}

pub fn seal(keys: &[&age::x25519::Identity], bytes: &[u8]) -> Vec<u8> {
    let list: Vec<crate::secrets::Recipient> = keys
        .iter()
        .map(|key| crate::secrets::Recipient {
            name: "key".into(),
            key: key.to_public().to_string(),
        })
        .collect();
    crate::secrets::encrypt_for(&list, bytes).unwrap()
}

pub fn machine(fleet: &Path, name: &str, text: &str) {
    let path = crate::fleet::machines::path(fleet, name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

pub fn empty_fleet(dir: &Path) -> Fleet {
    name_recovery(dir);
    let type_dir = dir.join("types/workstation");
    fs::create_dir_all(type_dir.join("etc")).unwrap();
    fs::write(type_dir.join(SYSTEM_TOML), "").unwrap();
    let repo = Repo::new(dir.to_path_buf(), "workstation", &[], &Default::default()).unwrap();
    Fleet::of(repo, &current_user())
}
