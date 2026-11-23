use super::{
    bootstrap,
    net::{self, Needs, Remote, Scheme},
};
use crate::{
    package::Distro,
    system::user::User,
    trust::github,
    trust::ssh,
    util::cmd::{self, As},
    util::file,
    util::prompt::Prompt,
};
use anyhow::{Context, Result, bail};
use std::{fs, path::Path};

const NEW_KEY_PAGE: &str = "https://github.com/settings/ssh/new";
pub const KNOWN_HOSTS: &str = "/etc/ssh/ssh_known_hosts";

fn check_url(url: &str) -> Result<Option<Remote<'_>>> {
    match net::remote(url)? {
        Some(remote) if remote.scheme != Scheme::Ssh => bail!(
            "{url} isn't an SSH URL: give one such as git@github.com:you/fleet.git, so git reads \
             the fleet with this machine's own key"
        ),
        remote => Ok(remote),
    }
}

fn known_hosts_lines(known: &str, lines: &[String]) -> String {
    lines
        .iter()
        .filter(|line| !known.lines().any(|known| known == *line))
        .map(|line| format!("{line}\n"))
        .collect()
}

fn know_github() -> Result<()> {
    let text = net::retry(
        "Reading GitHub's host keys",
        &Needs::url(github::META)?,
        || cmd::bytes(As::Root, "curl", &["-fsSL", github::META]),
    )?;
    let path = Path::new(KNOWN_HOSTS);
    let known = file::read_if_any(path)?.unwrap_or_default();
    let added = known_hosts_lines(&known, &github::known_hosts(&text)?);
    if !added.is_empty() {
        file::write(path, format!("{known}{added}").as_bytes(), 0o644)?;
    }
    Ok(())
}

fn lets_in(url: &str, owner: &User) -> Result<bool> {
    let mut probe = As::User(owner).command("git", &["ls-remote", "--", url]);
    probe
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    let out = probe.output().context("running git")?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    match out.status.success() {
        true => Ok(true),
        false if stderr.contains("Permission denied (") => Ok(false),
        false if stderr.contains("Host key verification failed") => bail!(
            "this machine doesn't know {url}'s host key: add it to {KNOWN_HOSTS}, after checking \
             it, and run install again"
        ),
        false => bail!("git can't read {url}: {}", stderr.trim()),
    }
}

pub fn clone(
    url: &str,
    dir: &Path,
    owner: &User,
    title: &str,
    distro: Distro,
    prompt: &mut Prompt,
) -> Result<()> {
    let remote = check_url(url)?;
    let needs = Needs::url(url)?;
    if let Some(remote) = remote {
        bootstrap::ssh(distro)?;
        if remote.host == github::HOST {
            know_github()?;
        }
        if ssh::user_key(owner)?.is_none() {
            ssh::create_key(owner)?;
        }
        while !net::retry("Reaching the fleet", &needs, || lets_in(url, owner))? {
            let public = ssh::user_key(owner)?.context("install made the key")?;
            println!("{} doesn't let this machine's key in yet.", remote.host);
            if remote.host == github::HOST {
                println!("On a machine logged in to gh, run:");
                println!("  echo '{public}' | gh ssh-key add - --title {title}");
                println!("or paste it at {NEW_KEY_PAGE}, titled {title}.");
            } else {
                println!("Let it in for the fleet's repository: {public}");
            }
            if !prompt.confirm("Is it there now")? {
                bail!("install stopped before it could read the fleet");
            }
        }
    }
    let path = dir.to_string_lossy();
    net::retry("Cloning the fleet", &needs, || {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            match path.is_dir() {
                true => fs::remove_dir_all(path)?,
                false => fs::remove_file(path)?,
            }
        }
        cmd::run(As::User(owner), "git", &["clone", "--", url, &path])
    })
}

#[cfg(test)]
#[path = "../../tests/install/repo.rs"]
mod tests;
