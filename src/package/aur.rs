use crate::{
    cmd::{self, As},
    user::User,
    walk,
};
use anyhow::Result;
use serde::Deserialize;
use std::{ffi::CStr, fmt, fs, path::Path};

pub const URL: &str = "https://aur.archlinux.org";

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Helper {
    Yay,
    Paru,
}

impl Helper {
    pub fn command(self) -> &'static str {
        match self {
            Helper::Yay => "yay",
            Helper::Paru => "paru",
        }
    }

    fn package(self) -> &'static str {
        match self {
            Helper::Yay => "yay-bin",
            Helper::Paru => "paru-bin",
        }
    }

    pub fn installed(self, admin: &User) -> bool {
        cmd::found(As::User(admin), self.command())
    }

    pub fn build(self, admin: &User) -> Result<()> {
        let build = tempfile::tempdir()?;
        admin.own(build.path())?;
        let dir = build
            .path()
            .join(self.package())
            .to_string_lossy()
            .into_owned();
        let url = format!("{URL}/{}.git", self.package());
        cmd::run(As::User(admin), "git", &["clone", "--", &url, &dir])?;
        let make = "cd \"$1\" && makepkg --syncdeps --install --rmdeps --noconfirm";
        cmd::run(As::User(admin), "sh", &["-c", make, "sh", &dir])
    }
}

impl fmt::Display for Helper {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.command())
    }
}

const NO_PASSWORD: &CStr = c"/etc/sudoers.d/zz-mothership-install";
const LEAVING: [libc::c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

fn no_password_path() -> &'static Path {
    Path::new(NO_PASSWORD.to_str().expect("the path is ASCII"))
}

pub struct NoPassword([libc::sighandler_t; 3]);

extern "C" fn remove_and_leave(signal: libc::c_int) {
    unsafe {
        libc::unlink(NO_PASSWORD.as_ptr());
        libc::signal(signal, libc::SIG_DFL);
        libc::raise(signal);
    }
}

impl NoPassword {
    pub fn remove_leftover() -> Result<()> {
        walk::remove_if_found(no_password_path())?;
        Ok(())
    }

    pub fn grant(admin: &User) -> Result<NoPassword> {
        let handler = remove_and_leave as extern "C" fn(libc::c_int) as libc::sighandler_t;
        let previous = LEAVING.map(|signal| unsafe { libc::signal(signal, handler) });
        let granted = NoPassword(previous);
        let rule = format!("{} ALL=(root) NOPASSWD: /usr/bin/pacman\n", admin.name);
        walk::write(no_password_path(), rule.as_bytes(), 0o440)?;
        Ok(granted)
    }
}

impl Drop for NoPassword {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_file(no_password_path()) {
            eprintln!("Remove {} by hand: {e}", no_password_path().display());
        }
        for (signal, previous) in LEAVING.into_iter().zip(self.0) {
            unsafe { libc::signal(signal, previous) };
        }
    }
}
