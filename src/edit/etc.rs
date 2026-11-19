use super::doc::{self, edit_toml, find_array};
use crate::{
    file,
    fleet::{Fleet, Mode},
    hash,
    preview::Seen,
    secrets::{self, Storage},
};
use anyhow::{Context, Result, bail};
use std::{
    ffi::OsStr,
    fs,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Path, PathBuf},
};
use toml_edit::value;

pub fn copy(fleet: &Fleet, rel: &Path, path: &Path, storage: Storage, seen: &Seen) -> Result<()> {
    if fleet.etc_elsewhere.contains(rel) {
        bail!(
            "etc/{} belongs to hardware or an option this machine doesn't have",
            rel.display()
        );
    }
    let clear = fleet.etc_dir().join(rel);
    let mut sealed = clear.clone().into_os_string();
    sealed.push(secrets::AGE);
    let sealed = PathBuf::from(sealed);
    let (dest, other) = match storage {
        Storage::Clear => (clear.as_path(), sealed.as_path()),
        Storage::Encrypted => (sealed.as_path(), clear.as_path()),
    };
    fs::create_dir_all(dest.parent().expect("under etc/"))?;
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        bail!(
            "{} is a folder: copy the files you want one by one",
            path.display()
        );
    }
    let mut bytes = file::content(path)?;
    seen.check(&hash::sha256(&bytes))?;
    if meta.is_symlink() {
        if storage == Storage::Encrypted {
            bail!("{} is a link, which holds no secret", path.display());
        }
        file::link(dest, Path::new(OsStr::from_bytes(&bytes)))?;
    } else {
        if storage == Storage::Encrypted {
            if !fleet.etc_open() {
                bail!(
                    "root's key isn't in {}: `mothership trust` it first",
                    fleet.recipients().display()
                );
            }
            bytes = secrets::encrypt(&fleet.recipients(), &bytes)?;
        }
        let mode = meta.permissions().mode() & 0o7777;
        let executable = mode & 0o111 != 0;
        file::write(
            dest,
            &bytes,
            Mode::default_for(executable, Storage::Clear).0,
        )?;
        let default = Mode::default_for(executable, storage);
        let rel = rel.to_string_lossy().into_owned();
        edit_toml(&fleet.system_path(), |doc| {
            if Mode(mode) == default {
                if let Some(modes) = doc.get_mut("etc_modes").and_then(|m| m.as_table_like_mut()) {
                    modes.remove(&rel);
                }
                return Ok(());
            }
            doc.entry("etc_modes")
                .or_insert(toml_edit::table())
                .as_table_like_mut()
                .context("`etc_modes` in system.toml is not a table")?
                .insert(&rel, value(format!("{mode:o}")));
            Ok(())
        })?;
    }
    if file::remove_if_found(other)? && storage == Storage::Encrypted {
        println!("  git history keeps the version in clear, so change it.");
    }
    Ok(())
}

pub fn remove(fleet: &Fleet, rel: &Path) -> Result<()> {
    let dest = fleet.etc_source(rel);
    fs::remove_file(&dest).with_context(|| format!("removing {}", dest.display()))?;
    let root = fleet.etc_dir();
    for dir in dest.ancestors().skip(1).take_while(|dir| *dir != root) {
        if fs::read_dir(dir)?.next().is_some() {
            break;
        }
        fs::remove_dir(dir)?;
    }
    let rel = rel.to_string_lossy();
    edit_toml(&fleet.system_path(), |doc| {
        for table in &fleet.blocks {
            if let Some(array) = find_array(doc, table, &["etc"]) {
                doc::remove(array, &rel);
            }
        }
        if let Some(modes) = doc.get_mut("etc_modes").and_then(|m| m.as_table_like_mut()) {
            modes.remove(&rel);
        }
        Ok(())
    })
}
