use crate::{file, hash::Fields, user::User};
use anyhow::{Context, Result, bail};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt, symlink},
    },
    path::{Component, Path, PathBuf},
    sync::mpsc,
    thread,
};
use tempfile::TempDir;

const FILE_LIMIT: u64 = 64 << 20;

pub struct Snapshot {
    dir: TempDir,
    digest: String,
}

enum Entry {
    Dir { rel: PathBuf, mode: u32 },
    File { rel: PathBuf, mode: u32, file: File },
    Link { rel: PathBuf, target: PathBuf },
}

impl Snapshot {
    pub fn take(user: &User, targets: &[&Path]) -> Result<Snapshot> {
        let dir = TempDir::new()?;
        let home = dir.path().join("home");
        fs::create_dir(&home)?;
        let (tx, entries) = mpsc::sync_channel(16);
        let mut fields = Fields::new();
        let mut dirs = HashSet::new();
        thread::scope(|s| {
            let reader = s.spawn(move || {
                user.with_rights(|| {
                    let mut sent = HashSet::new();
                    targets
                        .iter()
                        .try_for_each(|target| send(&user.home, target, &mut sent, &tx))
                })
            });
            let written = entries.iter().try_for_each(|entry| {
                write(&home, entry, &mut dirs, &mut fields)?;
                Ok::<_, anyhow::Error>(())
            });
            drop(entries);
            let read = reader
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            written.and(read)
        })?;
        Ok(Snapshot {
            digest: fields.finish(),
            dir,
        })
    }

    pub fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn path(&self, user: &User, target: &Path) -> Result<PathBuf> {
        Ok(self.home().join(relative(&user.home, target)?))
    }
}

fn relative<'a>(home: &Path, target: &'a Path) -> Result<&'a Path> {
    let rel = target
        .strip_prefix(home)
        .with_context(|| format!("{} isn't in {}", target.display(), home.display()))?;
    if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        bail!("{} isn't a plain path in the home", target.display());
    }
    Ok(rel)
}

fn send(
    home: &Path,
    target: &Path,
    sent: &mut HashSet<PathBuf>,
    tx: &mpsc::SyncSender<Entry>,
) -> Result<()> {
    let rel = relative(home, target)?;
    let mut prefix = PathBuf::new();
    for part in rel.parent().into_iter().flat_map(Path::iter) {
        prefix.push(part);
        if !sent.insert(prefix.clone()) {
            continue;
        }
        let meta = fs::metadata(home.join(&prefix))?;
        if !meta.is_dir() {
            bail!("{} isn't a folder", home.join(&prefix).display());
        }
        let mode = meta.mode() & 0o777;
        emit(
            tx,
            Entry::Dir {
                rel: prefix.clone(),
                mode,
            },
        )?;
    }
    copy(target, rel, tx)
}

fn copy(path: &Path, rel: &Path, tx: &mpsc::SyncSender<Entry>) -> Result<()> {
    let Some(meta) = file::if_found(fs::symlink_metadata(path))
        .with_context(|| format!("reading {}", path.display()))?
    else {
        return Ok(());
    };
    let rel = rel.to_path_buf();
    let mode = meta.mode() & 0o777;
    if meta.is_symlink() {
        let target = fs::read_link(path)?;
        return emit(tx, Entry::Link { rel, target });
    }
    if meta.is_dir() {
        emit(
            tx,
            Entry::Dir {
                rel: rel.clone(),
                mode,
            },
        )?;
        for entry in fs::read_dir(path).with_context(|| format!("reading {}", path.display()))? {
            let entry = entry?;
            copy(&entry.path(), &rel.join(entry.file_name()), tx)?;
        }
        return Ok(());
    }
    if !meta.is_file() {
        return Ok(());
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let opened = file.metadata()?;
    if !opened.is_file() {
        bail!("{} changed while being read", path.display());
    }
    emit(tx, Entry::File { rel, mode, file })
}

fn emit(tx: &mpsc::SyncSender<Entry>, entry: Entry) -> Result<()> {
    tx.send(entry).context("the snapshot stopped")
}

fn write(
    home: &Path,
    entry: Entry,
    dirs: &mut HashSet<PathBuf>,
    fields: &mut Fields,
) -> Result<()> {
    let rel = match &entry {
        Entry::Dir { rel, .. } | Entry::File { rel, .. } | Entry::Link { rel, .. } => rel,
    };
    let parent = rel.parent().unwrap_or(Path::new(""));
    if !parent.as_os_str().is_empty() && !dirs.contains(parent) {
        bail!("~/{} came before its folder", rel.display());
    }
    let path = home.join(rel);
    match entry {
        Entry::Dir { rel, mode } => {
            fs::create_dir(&path)?;
            fs::set_permissions(&path, fs::Permissions::from_mode(mode))?;
            fields.add(b"d");
            fields.add(rel.as_os_str().as_bytes());
            fields.add(&mode.to_le_bytes());
            dirs.insert(rel);
        }
        Entry::Link { rel, target } => {
            symlink(&target, &path)?;
            fields.add(b"l");
            fields.add(rel.as_os_str().as_bytes());
            fields.add(target.as_os_str().as_bytes());
        }
        Entry::File { rel, mode, file } => {
            let mut bytes = Vec::new();
            file.take(FILE_LIMIT + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > FILE_LIMIT {
                bail!("~/{} is over {} MB", rel.display(), FILE_LIMIT >> 20);
            }
            let mut out = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            out.set_permissions(fs::Permissions::from_mode(mode))?;
            out.write_all(&bytes)?;
            fields.add(b"f");
            fields.add(rel.as_os_str().as_bytes());
            fields.add(&mode.to_le_bytes());
            fields.add(&bytes);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/snapshot.rs"]
mod tests;
