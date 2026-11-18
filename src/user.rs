use anyhow::{Context, Result, bail};
use std::{
    env, fs,
    os::unix,
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub struct User {
    pub name: String,
    uid: u32,
    gid: u32,
    pub home: PathBuf,
    pub shell: String,
}

static CURRENT: OnceLock<User> = OnceLock::new();

/// Under sudo, the user who ran it: apply still works on their home, files and services.
pub fn init() -> Result<()> {
    let name = if is_root() {
        env::var("SUDO_USER").context("run it with sudo from your own account")?
    } else {
        env::var("USER").context("USER is not set")?
    };
    let user = lookup(&name)?;
    CURRENT.get_or_init(|| user);
    Ok(())
}

pub fn current() -> &'static User {
    CURRENT.get().expect("main calls user::init first")
}

pub fn home() -> PathBuf {
    current().home.clone()
}

pub fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions and can't fail.
    unsafe { libc::geteuid() == 0 }
}

/// Under sudo, what review writes into the fleet checkout and the user's state stays the user's.
pub fn own(path: &Path) -> Result<()> {
    if is_root() {
        let user = current();
        unix::fs::lchown(path, Some(user.uid), Some(user.gid))
            .with_context(|| format!("giving {} to {}", path.display(), user.name))?;
    }
    Ok(())
}

/// Like `fs::create_dir_all`, with each new folder owned by the user.
pub fn create_dir_all(dir: &Path) -> Result<()> {
    let missing: Vec<&Path> = dir.ancestors().take_while(|d| !d.exists()).collect();
    fs::create_dir_all(dir)?;
    missing.into_iter().try_for_each(own)
}

/// Read directly: getent also asks systemd, which stalls while it lists unit files.
fn lookup(name: &str) -> Result<User> {
    let passwd = fs::read_to_string("/etc/passwd")?;
    for line in passwd.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.first() == Some(&name) {
            let [_, _, uid, gid, _, home, shell] = fields[..] else {
                bail!("{name}'s line in /etc/passwd is malformed");
            };
            return Ok(User {
                name: name.to_owned(),
                uid: uid.parse()?,
                gid: gid.parse()?,
                home: PathBuf::from(home),
                shell: shell.to_owned(),
            });
        }
    }
    bail!("{name} is not in /etc/passwd")
}
