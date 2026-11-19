use anyhow::{Context, Result, bail};
use std::{
    fs,
    io::{self, ErrorKind, Read, Write},
    os::unix::{
        ffi::OsStringExt,
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

pub const ETC: &str = "/etc";
const READ_LIMIT: u64 = 64 << 20;

pub fn if_found<T>(result: io::Result<T>) -> io::Result<Option<T>> {
    match result {
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        result => result.map(Some),
    }
}

pub fn remove_if_found(path: &Path) -> Result<bool> {
    Ok(if_found(fs::remove_file(path))
        .with_context(|| format!("removing {}", path.display()))?
        .is_some())
}

pub fn link(path: &Path, target: &Path) -> Result<()> {
    let dir = path.parent().context("no parent folder")?;
    fs::create_dir_all(dir)?;
    let name = path.file_name().context("no file name")?.to_string_lossy();
    let temp = dir.join(format!(".{name}.mothership"));
    remove_if_found(&temp)?;
    std::os::unix::fs::symlink(target, &temp)?;
    fs::rename(&temp, path)?;
    Ok(())
}

pub fn read_if_any(path: &Path) -> Result<Option<String>> {
    if_found(fs::read_to_string(path)).with_context(|| format!("reading {}", path.display()))
}

pub fn write(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let dir = path.parent().context("no parent folder")?;
    fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(bytes)?;
    let temp = file.as_file();
    if let Some(old) = if_found(fs::symlink_metadata(path))?
        && old.is_file()
    {
        let new = temp.metadata()?;
        if (old.uid(), old.gid()) != (new.uid(), new.gid()) {
            std::os::unix::fs::fchown(temp, Some(old.uid()), Some(old.gid()))
                .with_context(|| format!("keeping the owner of {}", path.display()))?;
        }
    }
    temp.set_permissions(fs::Permissions::from_mode(mode))?;
    temp.sync_all()?;
    file.persist(path)?;
    Ok(())
}

pub fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path);
    let Some(mut file) = if_found(file).with_context(|| format!("opening {}", path.display()))?
    else {
        return Ok(None);
    };
    let meta = file.metadata()?;
    if !meta.is_file() {
        bail!("{} isn't a regular file", path.display());
    }
    if meta.len() > READ_LIMIT {
        bail!("{} is over {} MB", path.display(), READ_LIMIT >> 20);
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(READ_LIMIT + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading {}", path.display()))?;
    if bytes.len() as u64 > READ_LIMIT {
        bail!("{} is over {} MB", path.display(), READ_LIMIT >> 20);
    }
    Ok(Some(bytes))
}

pub fn content(path: &Path) -> Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path).with_context(|| format!("reading {}", path.display()))?;
    if meta.is_symlink() {
        return Ok(fs::read_link(path)?.into_os_string().into_vec());
    }
    fs::read(path).with_context(|| format!("reading {}", path.display()))
}

pub fn under(root: &Path, absolute: &str) -> PathBuf {
    root.join(absolute.trim_start_matches('/'))
}

pub fn read_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> Result<T> {
    match read_if_any(path)? {
        Some(text) => {
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
        }
        None => Ok(T::default()),
    }
}

pub fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    write(path, serde_json::to_string_pretty(value)?.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
#[path = "../tests/file.rs"]
mod tests;
