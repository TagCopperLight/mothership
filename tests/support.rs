use crate::{
    fleet::{Fleet, MANIFEST, Repo, SYSTEM_TOML},
    item::{Item, Section},
    user::{self, User},
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
    let text = format!("[recovery]\nage = \"{}\"\n", key.to_public());
    fs::write(dir.join(MANIFEST), text).unwrap();
    key
}

pub fn seal(list: &Path, bytes: &[u8]) -> Vec<u8> {
    crate::secrets::encrypt_for(&crate::secrets::read(list).unwrap(), bytes).unwrap()
}

pub fn empty_fleet(dir: &Path) -> Fleet {
    name_recovery(dir);
    let type_dir = dir.join("types/workstation");
    fs::create_dir_all(type_dir.join("etc")).unwrap();
    fs::write(type_dir.join(SYSTEM_TOML), "").unwrap();
    let repo = Repo::new(dir.to_path_buf(), "workstation", &[], &Default::default()).unwrap();
    Fleet::of(repo, &current_user())
}
