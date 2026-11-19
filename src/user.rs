use crate::{cmd, fleet::Member};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    os::unix,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq)]
pub struct User {
    pub name: String,
    pub uid: u32,
    gid: u32,
    pub home: PathBuf,
    pub shell: String,
}

impl User {
    pub fn own(&self, path: &Path) -> Result<()> {
        unix::fs::lchown(path, Some(self.uid), Some(self.gid))
            .with_context(|| format!("giving {} to {}", path.display(), self.name))
    }

    pub fn own_all(&self, dir: &Path) -> Result<()> {
        let owner = format!("{}:{}", self.uid, self.gid);
        cmd::root(
            "chown",
            &["-R", "--no-dereference", &owner, &dir.to_string_lossy()],
        )
    }

    pub fn create_dir_all(&self, dir: &Path) -> Result<()> {
        let missing: Vec<&Path> = dir.ancestors().take_while(|d| !d.exists()).collect();
        fs::create_dir_all(dir)?;
        missing.into_iter().try_for_each(|d| self.own(d))
    }

    pub fn expand(&self, path: &str) -> PathBuf {
        match path.strip_prefix("~/") {
            Some(rest) => self.home.join(rest),
            None => PathBuf::from(path),
        }
    }

    pub fn tilde(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        }
    }
}

pub fn create(member: &Member) -> Result<()> {
    let mut args = vec!["--create-home"];
    if let Some(shell) = &member.config.shell {
        args.extend(["--shell", shell]);
    }
    args.push(&member.name);
    cmd::root("useradd", &args)
}

pub fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

pub fn by_name(name: &str) -> Result<Option<User>> {
    find(|user| user.name == name)
}

pub fn by_uid(uid: u32) -> Result<User> {
    find(|user| user.uid == uid)?.with_context(|| format!("no user has uid {uid}"))
}

fn find(wanted: impl Fn(&User) -> bool) -> Result<Option<User>> {
    let passwd = fs::read_to_string("/etc/passwd")?;
    for line in passwd.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        let [name, _, uid, gid, _, home, shell] = fields[..] else {
            bail!("a line in /etc/passwd is malformed: {line}");
        };
        let user = User {
            name: name.to_owned(),
            uid: uid.parse()?,
            gid: gid.parse()?,
            home: PathBuf::from(home),
            shell: shell.to_owned(),
        };
        if wanted(&user) {
            return Ok(Some(user));
        }
    }
    Ok(None)
}
