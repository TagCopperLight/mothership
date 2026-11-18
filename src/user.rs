use anyhow::{Context, Result, bail};
use std::{env, fs, path::PathBuf, sync::OnceLock};

pub struct User {
    pub name: String,
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

/// Read directly: getent also asks systemd, which stalls while it lists unit files.
fn lookup(name: &str) -> Result<User> {
    let passwd = fs::read_to_string("/etc/passwd")?;
    for line in passwd.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.first() == Some(&name) {
            let [.., home, shell] = fields[..] else {
                bail!("{name}'s line in /etc/passwd is malformed");
            };
            return Ok(User {
                name: name.to_owned(),
                home: PathBuf::from(home),
                shell: shell.to_owned(),
            });
        }
    }
    bail!("{name} is not in /etc/passwd")
}
