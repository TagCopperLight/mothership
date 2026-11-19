use crate::{cmd, fleet::Member, walk};
use anyhow::{Context, Result, bail};
use std::{
    fs, io,
    os::unix,
    path::{Path, PathBuf},
    thread,
};

const TOOL_BINS: [&str; 3] = [".cargo/bin", ".local/bin", ".bun/bin"];
pub const SYSTEM_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

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

    pub fn environment(&self) -> Vec<(&'static str, String)> {
        let bins = TOOL_BINS.map(|bin| self.home.join(bin).display().to_string());
        let mut env = vec![
            ("HOME", self.home.display().to_string()),
            ("USER", self.name.clone()),
            ("LOGNAME", self.name.clone()),
            ("SHELL", self.shell.clone()),
            ("PATH", format!("{SYSTEM_PATH}:{}", bins.join(":"))),
            ("LANG", "C.UTF-8".into()),
        ];
        let runtime = format!("/run/user/{}", self.uid);
        if Path::new(&runtime).is_dir() {
            env.push(("XDG_RUNTIME_DIR", runtime));
        }
        env
    }

    pub fn with_rights<T: Send>(&self, work: impl FnOnce() -> Result<T> + Send) -> Result<T> {
        let groups = groups_of(&self.name, self.gid)?;
        thread::scope(|s| {
            s.spawn(|| {
                self.take_rights(&groups)?;
                work()
            })
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        })
    }

    fn take_rights(&self, groups: &[libc::gid_t]) -> Result<()> {
        if unsafe { libc::geteuid() } == self.uid {
            return Ok(());
        }
        unsafe {
            if libc::syscall(libc::SYS_setgroups, groups.len(), groups.as_ptr()) != 0 {
                return Err(io::Error::last_os_error()).context("dropping root's groups");
            }
            libc::syscall(libc::SYS_setfsgid, self.gid);
            libc::syscall(libc::SYS_setfsuid, self.uid);
            let gid = libc::syscall(libc::SYS_setfsgid, u32::MAX) as u32;
            let uid = libc::syscall(libc::SYS_setfsuid, u32::MAX) as u32;
            if (uid, gid) != (self.uid, self.gid) {
                bail!("couldn't take {}'s rights to read their files", self.name);
            }
        }
        Ok(())
    }

    pub fn read(&self, path: &Path) -> Result<Option<Vec<u8>>> {
        self.with_rights(|| walk::read(path))
    }

    pub fn write(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.with_rights(|| walk::write(path, bytes, 0o600))
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
    args.extend(["--", &member.name]);
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

pub struct Group {
    pub name: String,
    gid: libc::gid_t,
    members: Vec<String>,
}

pub fn groups() -> Result<Vec<Group>> {
    fs::read_to_string("/etc/group")?
        .lines()
        .map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            let [name, _, gid, members] = fields[..] else {
                bail!("a line in /etc/group is malformed: {line}");
            };
            Ok(Group {
                name: name.to_owned(),
                gid: gid.parse()?,
                members: members
                    .split(',')
                    .filter(|m| !m.is_empty())
                    .map(str::to_owned)
                    .collect(),
            })
        })
        .collect()
}

pub fn joined(user: &str) -> Result<Vec<Group>> {
    Ok(groups()?
        .into_iter()
        .filter(|group| group.members.iter().any(|m| m == user))
        .collect())
}

fn groups_of(name: &str, gid: libc::gid_t) -> Result<Vec<libc::gid_t>> {
    let mut gids: Vec<libc::gid_t> = joined(name)?.iter().map(|g| g.gid).collect();
    gids.push(gid);
    gids.sort_unstable();
    gids.dedup();
    Ok(gids)
}
