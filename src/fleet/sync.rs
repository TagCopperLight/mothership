use super::{
    checkout::{DIR, in_checkout, move_main},
    git::{GIT_DIR, Git, short},
    recipients,
};
use crate::{
    services::confirm,
    system::user::User,
    trust::sign::Signer,
    trust::verify::{self, Refused},
    util::cmd::{self, As},
    util::file,
};
use anyhow::{Context, Result, bail};
use std::{fs, path::Path};

pub const CURRENT: &str = "/var/lib/mothership/current";
pub const CHECKED: &str = "refs/mothership/checked";
pub const ORIGIN: &str = "refs/incoming/origin/main";
const EXPORTS: &str = "/var/lib/mothership/checked";
const MAIN: &str = "refs/incoming/heads/main";

pub struct Sync {
    pub refused: Option<Refused>,
}

#[derive(Clone, Copy)]
pub enum Remote {
    Fetch,
    Skip,
}

pub enum Anchor<'a> {
    Kept,
    Code(&'a str),
    SignedBy(&'a Signer),
    Tip,
}

pub fn accepted(git: &Git) -> Result<String> {
    git.rev(CHECKED)?
        .context("this machine trusts no commit of the fleet yet")
}

pub fn anchored() -> Result<bool> {
    Ok(Path::new(GIT_DIR).exists() && Git::at(GIT_DIR).rev(CHECKED)?.is_some())
}

pub fn sync(admin: &User, remote: Remote) -> Result<Sync> {
    sync_from(admin, remote, Anchor::Kept)
}

pub fn sync_from(admin: &User, remote: Remote, anchor: Anchor) -> Result<Sync> {
    let git = Git::at(GIT_DIR);
    let (main, tip) = fetch(&git, admin, remote)?;
    let before = git.rev(CHECKED)?;
    let fresh = before.is_none() || matches!(anchor, Anchor::Tip);
    let anchor = match (anchor, before) {
        (Anchor::Kept, Some(checked)) => checked,
        (Anchor::Kept, None) => {
            bail!("this machine trusts no commit of the fleet yet: run `sudo mothership install`")
        }
        (Anchor::Code(code), checked) => {
            let commit = git
                .rev(&format!("{code}^{{commit}}"))?
                .with_context(|| format!("the fleet has no commit {code}"))?;
            if !git.is_ancestor(&commit, &tip)? {
                bail!("{code} isn't on the fleet's main");
            }
            checked.unwrap_or(commit)
        }
        (Anchor::SignedBy(_), Some(checked)) => checked,
        (Anchor::SignedBy(signer), None) => newest_signed(&git, &tip, signer)?,
        (Anchor::Tip, _) => tip.clone(),
    };
    if fresh {
        confirm::note_inherited(&git, &anchor)?;
    }
    let checked = verify::check(&git, &anchor, &tip)?;
    confirm::note_unconfirmed(&git, &checked.steps)?;
    git.out(&["update-ref", CHECKED, &checked.commit])?;
    export(&git, &checked.commit)?;
    if checked.commit != main
        && git.is_ancestor(&main, &checked.commit)?
        && let Err(e) = move_main(admin, &main, &checked.commit)
    {
        println!(
            "Couldn't move main to {} in {DIR}: {e:#}",
            short(&checked.commit)
        );
    }
    Ok(Sync {
        refused: checked.refused,
    })
}

fn newest_signed(git: &Git, tip: &str, signer: &Signer) -> Result<String> {
    for commit in git.out(&["rev-list", "--first-parent", tip])?.lines() {
        if git
            .commit(commit)?
            .signer(std::slice::from_ref(signer))
            .is_some()
        {
            return Ok(commit.to_owned());
        }
    }
    bail!(
        "no commit on the fleet's main is signed by the {} key, so nothing tells which commits \
         to trust: trust this machine from one that signs for the fleet",
        signer.name
    )
}

fn fetch(git: &Git, admin: &User, remote: Remote) -> Result<(String, String)> {
    if !Path::new(GIT_DIR).exists() {
        cmd::root("git", &["init", "--quiet", "--bare", GIT_DIR])?;
        git.out(&["config", "fetch.fsckObjects", "true"])?;
    }
    if let Remote::Fetch = remote
        && let Err(e) = in_checkout(admin, &["fetch", "--quiet", "origin"])
    {
        println!("Couldn't fetch the fleet, so this works from what's here: {e:#}");
    }
    let upload = format!(
        "setpriv --reuid={} --regid={} --init-groups -- git-upload-pack",
        admin.uid, admin.gid
    );
    let args = [
        "fetch",
        "--quiet",
        "--no-tags",
        "--prune",
        "--upload-pack",
        &upload,
        DIR,
        "+refs/heads/*:refs/incoming/heads/*",
        "+refs/remotes/origin/*:refs/incoming/origin/*",
    ];
    git.out(&args)?;
    tips(git)
}

pub fn tips(git: &Git) -> Result<(String, String)> {
    let main = git
        .rev(MAIN)?
        .with_context(|| format!("{DIR} has no main branch"))?;
    let tip = match git.rev(ORIGIN)? {
        Some(origin) if git.is_ancestor(&main, &origin)? => origin,
        Some(origin) if git.is_ancestor(&origin, &main)? => main.clone(),
        Some(_) => bail!("main and origin/main have split in {DIR}: merge or rebase them"),
        None => main.clone(),
    };
    Ok((main, tip))
}

pub fn newest(admin: &User, count: usize) -> Result<Vec<String>> {
    let git = Git::at(GIT_DIR);
    let (_, tip) = fetch(&git, admin, Remote::Fetch)?;
    let list = git.out(&[
        "rev-list",
        "--first-parent",
        &format!("--max-count={count}"),
        &tip,
    ])?;
    let mut lines = Vec::new();
    for commit in list.lines() {
        let line = git.out(&["log", "-1", "--format=%an: %s", commit])?;
        let parent = git.rev(&format!("{commit}^1"))?;
        let by = match parent {
            Some(parent) => match verify::verify(&git, &parent, commit) {
                Ok(Some(signer)) => format!("signed by {}", signer.name),
                Ok(None) => "signed by nobody, as the fleet named no signer".to_owned(),
                Err(why) => format!("{why:#}"),
            },
            None => "the fleet's first commit".to_owned(),
        };
        lines.push(format!("{} {line}  ({by})", short(commit)));
    }
    Ok(lines)
}

pub fn exported() -> Result<Option<String>> {
    Ok(file::if_found(fs::read_link(CURRENT))
        .with_context(|| format!("reading {CURRENT}"))?
        .and_then(|target| target.file_name().map(|n| n.to_string_lossy().into_owned())))
}

fn export(git: &Git, commit: &str) -> Result<()> {
    if exported()?.as_deref() == Some(commit) {
        return Ok(());
    }
    let exports = Path::new(EXPORTS);
    fs::create_dir_all(exports)?;
    let done = exports.join(commit);
    if !done.exists() {
        extract(git, commit, &done)?;
    }
    recipients::write_lists(&done)?;
    file::link(Path::new(CURRENT), &done)?;
    for entry in fs::read_dir(exports)? {
        let path = entry?.path();
        if path != done {
            fs::remove_dir_all(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }
    Ok(())
}

pub fn extract(git: &Git, commit: &str, done: &Path) -> Result<()> {
    let partial = done.with_file_name(".partial");
    if partial.exists() {
        fs::remove_dir_all(&partial)?;
    }
    fs::create_dir(&partial)?;
    let tar = cmd::output(git.command(&["archive", commit]), "git", &["archive"])?;
    let into = partial.to_string_lossy();
    cmd::with_input(
        As::Root,
        "tar",
        &["-x", "--no-same-owner", "-C", &into],
        &tar,
    )?;
    fs::rename(&partial, done)?;
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/fleet/sync.rs"]
mod tests;
