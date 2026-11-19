use crate::{
    fleet::{Fleet, Repo, SYSTEM_TOML},
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

pub fn empty_fleet(dir: &Path) -> Fleet {
    let type_dir = dir.join("types/workstation");
    fs::create_dir_all(type_dir.join("etc")).unwrap();
    fs::write(type_dir.join(SYSTEM_TOML), "").unwrap();
    let repo = Repo::new(dir.to_path_buf(), "workstation", &[], &Default::default()).unwrap();
    Fleet::of(repo, &current_user())
}
