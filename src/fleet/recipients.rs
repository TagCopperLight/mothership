use super::{
    Manifest,
    homes::{self, Reader},
    machines::Machines,
};
use crate::{secrets, util::file};
use anyhow::Result;
use std::{
    fs,
    path::{Path, PathBuf},
};

const LISTS: &str = "/var/lib/mothership/recipients";

pub fn list_path(type_name: &str, user: &str) -> PathBuf {
    Path::new(LISTS).join(type_name).join(format!("{user}.txt"))
}

pub fn write_lists(tree: &Path) -> Result<()> {
    let machines = Machines::load(tree)?;
    let recovery = Manifest::read(tree)?.recovery();
    let partial = Path::new(LISTS).with_file_name(".recipients");
    if partial.exists() {
        fs::remove_dir_all(&partial)?;
    }
    for reader in homes::readers(tree)? {
        let Reader::Home(home) = &reader else {
            continue;
        };
        let keys: Vec<String> = machines
            .sealed(std::slice::from_ref(&reader), recovery.as_ref())
            .into_iter()
            .map(|recipient| recipient.key)
            .collect();
        let path = partial
            .join(&home.type_name)
            .join(format!("{}.txt", home.user));
        file::write(&path, secrets::list_text(&keys).as_bytes(), 0o644)?;
    }
    if Path::new(LISTS).exists() {
        fs::remove_dir_all(LISTS)?;
    }
    if partial.exists() {
        fs::rename(&partial, LISTS)?;
    }
    Ok(())
}
