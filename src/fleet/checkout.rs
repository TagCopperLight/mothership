use crate::{
    system::user::{self, User},
    util::cmd::{self, As},
    util::style,
};
use anyhow::{Context, Result};
use std::{fs, os::unix::fs::MetadataExt, path::Path, process::Command};

pub const DIR: &str = "/var/lib/mothership/fleet";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Unsaved {
    Everything,
    Uncommitted,
}

pub fn admin() -> Result<User> {
    let meta = fs::metadata(DIR)
        .with_context(|| format!("reading {DIR}: clone the fleet there as its admin"))?;
    user::by_uid(meta.uid())
}

pub fn require_admin_sudo() -> Result<()> {
    require_sudo(&admin()?)
}

pub fn require_sudo(admin: &User) -> Result<()> {
    cmd::bytes(
        As::Root,
        "sudo",
        &["-n", "-l", "-U", &admin.name, "/bin/sh"],
    )
    .map(drop)
    .with_context(|| {
        format!(
            "{} can't use sudo, so they must not own the fleet root applies",
            admin.name
        )
    })
}

pub fn unsaved(admin: &User, which: Unsaved) -> Result<Vec<String>> {
    let status = in_checkout(admin, &["status", "--porcelain=v2", "--branch"])?;
    let changed = status.lines().filter(|l| !l.starts_with('#')).count();
    let ahead: usize = status
        .lines()
        .find_map(|l| l.strip_prefix("# branch.ab +"))
        .and_then(|ab| ab.split_whitespace().next())
        .map(str::parse)
        .transpose()?
        .unwrap_or(0);
    let mut unsaved = Vec::new();
    let everything = which == Unsaved::Everything;
    if everything && !status.lines().any(|l| l.starts_with("# branch.upstream ")) {
        unsaved.push("no upstream branch to push to".to_owned());
    }
    if changed > 0 {
        unsaved.push(format!("{} not committed", style::count(changed, "change")));
    }
    if everything && ahead > 0 {
        unsaved.push(format!("{} not pushed", style::count(ahead, "commit")));
    }
    Ok(unsaved)
}

pub fn in_checkout(admin: &User, args: &[&str]) -> Result<String> {
    cmd::stdout(As::User(admin), "git", &[&["-C", DIR][..], args].concat())
}

pub fn author(admin: &User) -> Result<String> {
    author_if_set(admin)?.with_context(|| {
        format!(
            "{} has no git user.name and user.email: set them in their ~/.gitconfig",
            admin.name
        )
    })
}

pub fn author_if_set(admin: &User) -> Result<Option<String>> {
    author_from(|key| git_config(As::User(admin), &["-C", DIR, "config", key]))
}

pub fn author_from(read: impl Fn(&str) -> Result<Option<String>>) -> Result<Option<String>> {
    Ok(match (read("user.name")?, read("user.email")?) {
        (Some(name), Some(email)) => Some(format!("{name} <{email}>")),
        _ => None,
    })
}

pub fn git_config(who: As, args: &[&str]) -> Result<Option<String>> {
    let value = cmd::bytes_allowing_exit(who, "git", args, 1)?;
    let value = String::from_utf8(value)?.trim().to_owned();
    Ok((!value.is_empty()).then_some(value))
}

pub fn origin(admin: &User) -> Result<String> {
    origin_of(Path::new(DIR), || {
        As::User(admin).command::<&str>("git", &[])
    })
}

pub fn origin_of(dir: &Path, git: impl Fn() -> Command) -> Result<String> {
    let path = dir.to_string_lossy();
    let get = ["-C", &path, "remote", "get-url", "origin"];
    let mut command = git();
    command.args(get);
    Ok(String::from_utf8(cmd::output(command, "git", &get)?)?
        .trim()
        .to_owned())
}

pub fn moved() -> anyhow::Error {
    anyhow::anyhow!("main moved in {DIR} while mothership worked: run it again")
}

pub fn move_main(admin: &User, from: &str, to: &str) -> Result<()> {
    let git = |args: &[&str]| in_checkout(admin, args);
    match git(&["branch", "--show-current"])?.trim() {
        "main" if git(&["rev-parse", "HEAD"])?.trim() != from => Err(moved()),
        "main" => git(&["update-index", "-q", "--refresh"])
            .and_then(|_| git(&["reset", "--keep", "--quiet", to])),
        _ => git(&["update-ref", "refs/heads/main", to, from]),
    }
    .map(drop)
}
