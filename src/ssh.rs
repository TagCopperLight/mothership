use crate::{file, user::User};
use anyhow::{Context, Result, anyhow};
use ssh_key::{Algorithm, LineEnding, PrivateKey, PublicKey, rand_core::OsRng};
use std::{fs, io::ErrorKind, os::unix::fs::DirBuilderExt, path::PathBuf};

pub const KEYS: &str = "ssh-keys.txt";

pub fn key_path(user: &User) -> PathBuf {
    user.home.join(".ssh/id_ed25519")
}

pub fn public_path(user: &User) -> PathBuf {
    key_path(user).with_extension("pub")
}

pub fn user_key(user: &User) -> Result<Option<String>> {
    let path = key_path(user);
    let Some(bytes) = user.read(&path)? else {
        return Ok(None);
    };
    let key = PrivateKey::from_openssh(bytes).map_err(|e| anyhow!("{}: {e}", path.display()))?;
    Ok(Some(key.public_key().to_openssh()?))
}

pub fn create_key(user: &User) -> Result<()> {
    write_key(user, &PrivateKey::random(&mut OsRng, Algorithm::Ed25519)?)
}

pub fn write_key(user: &User, key: &PrivateKey) -> Result<()> {
    let public = format!("{}\n", key.public_key().to_openssh()?);
    let path = key_path(user);
    user.with_rights(|| {
        let dir = path.parent().context("the key has no folder")?;
        match fs::DirBuilder::new().mode(0o700).create(dir) {
            Err(e) if e.kind() != ErrorKind::AlreadyExists => {
                return Err(e).with_context(|| format!("creating {}", dir.display()));
            }
            _ => {}
        }
        file::create_private(&path, key.to_openssh(LineEnding::LF)?.as_bytes())?;
        file::write(&public_path(user), public.as_bytes(), 0o644)
    })
}

pub fn normalize(text: &str) -> Result<String> {
    let mut key = PublicKey::from_openssh(text.trim()).map_err(|e| anyhow!("{text}: {e}"))?;
    key.set_comment("");
    Ok(key.to_openssh()?)
}
