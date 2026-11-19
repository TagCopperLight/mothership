use super::{
    bootstrap,
    net::{self, Needs, Scheme},
};
use crate::{
    cmd::{self, As, Cmd},
    package::Distro,
};
use anyhow::{Context, Result, bail};
use std::{fs, path::Path};

pub const GITHUB: &str = "https://github.com";
const GH_LOGIN: [&str; 4] = ["auth", "login", "--hostname", "github.com"];
const GH_CREDENTIALS: [&str; 4] = [
    "-c",
    "credential.helper=",
    "-c",
    "credential.helper=!gh auth git-credential",
];

fn check_url(url: &str) -> Result<()> {
    let Some(remote) = net::remote(url)? else {
        return Ok(());
    };
    if remote.scheme != Scheme::Https {
        bail!(
            "{url} needs an SSH key or sends nothing encrypted, and a new machine has no SSH key \
             yet: give the https URL"
        );
    }
    if remote.userinfo.is_some() {
        bail!(
            "{url} holds a login, and git would save it where every user of the machine reads \
             it: give the URL without it, and install logs in to GitHub with gh"
        );
    }
    Ok(())
}

fn on_github(url: &str) -> bool {
    url.starts_with(&format!("{GITHUB}/"))
}

enum Access {
    Open,
    NeedsLogin,
}

fn probe(url: &str, who: As) -> Result<Access> {
    let args = ["ls-remote", "--", url];
    let mut probe = who.command("git", &args);
    probe
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    let out = probe.output().context("running git")?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    match out.status.success() {
        true => Ok(Access::Open),
        false if on_github(url) && stderr.contains("terminal prompts disabled") => {
            Ok(Access::NeedsLogin)
        }
        false => bail!("git can't read {url}: {}", stderr.trim()),
    }
}

pub fn clone(url: &str, dir: &Path, who: As, distro: Distro) -> Result<bool> {
    check_url(url)?;
    let needs = Needs::url(url)?;
    let path = dir.to_string_lossy();
    let empty = || -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
        }
        Ok(())
    };
    let access = net::retry("Reaching the fleet", &needs, || probe(url, who))?;
    if let Access::Open = access {
        net::retry("Cloning the fleet", &needs, || {
            empty()?;
            cmd::run(who, "git", &["clone", "--", url, &path])
        })?;
        return Ok(false);
    }
    bootstrap::gh(distro)?;
    if cmd::bytes(who, "gh", &["auth", "token"]).is_err() {
        println!("The fleet is private: log in to GitHub.");
        let args = [&GH_LOGIN[..], &["--git-protocol", "https", "--web"]].concat();
        cmd::run(who, "gh", &args)?;
    }
    net::retry("Cloning the fleet", &needs, || {
        empty()?;
        cmd::run(who, "gh", &["repo", "clone", url, &path])
    })?;
    Ok(true)
}

pub fn log_in(who: As, token: &str) -> Result<()> {
    let args = [&GH_LOGIN[..], &["--with-token"]].concat();
    net::retry("Logging in to GitHub", &Needs::url(GITHUB)?, || {
        cmd::with_input(who, "gh", &args, token.as_bytes())
    })
}

pub fn origin(dir: &Path, git: impl Fn() -> Cmd) -> Result<String> {
    let path = dir.to_string_lossy();
    let get = ["-C", &path, "remote", "get-url", "origin"];
    let mut command = git();
    command.args(get);
    Ok(String::from_utf8(cmd::output(command, "git", &get)?)?
        .trim()
        .to_owned())
}

pub fn pull(dir: &Path, expected: Option<&str>, git: impl Fn() -> Cmd) -> Result<()> {
    let path = dir.to_string_lossy();
    let origin = origin(dir, &git)?;
    let origin = origin.as_str();
    if let Some(url) = expected
        && origin != url
    {
        bail!(
            "{} comes from {origin}, not {url}: run install with --fleet {origin}",
            dir.display()
        );
    }
    let args = [&GH_CREDENTIALS[..], &["-C", &path, "pull", "--ff-only"]].concat();
    net::retry("Pulling the fleet", &Needs::url(origin)?, || {
        let mut command = git();
        command.args(&args);
        cmd::status(command, "git", &args)
    })
}

#[cfg(test)]
#[path = "../../tests/repo.rs"]
mod tests;
