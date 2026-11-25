use super::{
    checkout::{self, DIR, in_checkout, move_main, moved},
    git::{GIT_DIR, Git, short},
    sync::{self, CHECKED, ORIGIN, Remote},
};
use crate::{
    system::user::User,
    trust::github,
    trust::sign::{self, Signer},
    trust::verify,
    util::cmd::{self, As},
    util::prompt::Prompt,
    util::style,
};
use anyhow::{Context, Result, bail};
use ssh_key::PrivateKey;
use std::{
    ffi::{OsStr, OsString},
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub const STAGING: &str = "/var/lib/mothership/staging";
const STAGED: &str = "refs/mothership/staged";

pub struct Stage {
    pub dir: PathBuf,
    base: String,
    accepted: String,
    index: tempfile::TempDir,
    kept: String,
}

#[derive(Clone, Copy)]
enum Settle {
    Vouch,
    Drop,
    Nothing,
}

pub fn stage(prompt: &mut Prompt) -> Result<Option<Stage>> {
    let git = Git::at(GIT_DIR);
    let accepted = git
        .rev(CHECKED)?
        .context("this machine has checked no commit of the fleet yet")?;
    let (main, tip) = sync::tips(&git)?;
    verify::follows(&git, &accepted, &tip)?;
    let base = match tip == accepted {
        true => accepted.clone(),
        false => match settle(&git, &accepted, &tip, prompt)? {
            Settle::Nothing => {
                println!("Nothing changed.");
                return Ok(None);
            }
            Settle::Vouch => {
                if main != tip {
                    move_main(&checkout::admin()?, &main, &tip)?;
                }
                tip
            }
            Settle::Drop => {
                drop_after(&git, &checkout::admin()?, &accepted, &main)?;
                accepted.clone()
            }
        },
    };
    let dir = PathBuf::from(STAGING);
    if dir.exists() {
        fs::remove_dir_all(&dir).with_context(|| format!("removing {STAGING}"))?;
    }
    sync::extract(&git, &base, &dir)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    let stage = Stage {
        dir,
        kept: git.out(&["rev-parse", &format!("{base}^{{tree}}")])?,
        base,
        accepted,
        index: tempfile::TempDir::new()?,
    };
    stage.staged(&["read-tree", &stage.base])?;
    Ok(Some(stage))
}

fn settle(git: &Git, accepted: &str, tip: &str, prompt: &mut Prompt) -> Result<Settle> {
    let range = format!("{accepted}..{tip}");
    let count: usize = git
        .out(&["rev-list", "--count", "--first-parent", &range])?
        .parse()?;
    println!(
        "main holds {} after {}, the last one this machine accepts:",
        style::count(count, "commit"),
        short(accepted)
    );
    let log = git.out(&[
        "log",
        "--first-parent",
        "--abbrev=12",
        "--format=  %h %an, %as: %s",
        &range,
    ])?;
    for line in log.lines() {
        println!("{}", style::clean(line));
    }
    println!("They change:");
    print_files(git, accepted, tip)?;
    if let Some(repo) = github::repo(&checkout::origin(&checkout::admin()?)?) {
        println!("Read them at https://github.com/{repo}/compare/{accepted}...{tip}");
    }
    let answers = [
        ('v', Settle::Vouch),
        ('d', Settle::Drop),
        ('n', Settle::Nothing),
    ];
    let question = "v vouch for them, signing on top · d drop them from main · n change nothing ›";
    Ok(prompt.ask(question, &answers)?.unwrap_or(Settle::Nothing))
}

fn print_files(git: &Git, base: &str, commit: &str) -> Result<()> {
    let args = [
        "-c",
        "core.quotePath=false",
        "diff-tree",
        "-r",
        "--name-status",
        base,
        commit,
    ];
    let files = cmd::output(git.command(&args), "git", &args)?;
    for line in String::from_utf8_lossy(&files).lines() {
        println!("  {}", style::clean(&line.replacen('\t', " ", 1)));
    }
    Ok(())
}

fn drop_after(git: &Git, admin: &User, accepted: &str, main: &str) -> Result<()> {
    if main != accepted {
        move_main(admin, main, accepted)?;
    }
    if let Some(origin) = git.rev(ORIGIN)?
        && !git.is_ancestor(&origin, accepted)?
    {
        in_checkout(
            admin,
            &[
                "push",
                "--quiet",
                &format!("--force-with-lease=main:{origin}"),
                "origin",
                &format!("{accepted}:refs/heads/main"),
            ],
        )?;
    }
    Ok(())
}

pub enum Outcome {
    Unchanged(String),
    Declined,
    Pushed(String),
}

#[derive(Clone, Copy)]
pub enum Advance {
    Merge,
    KeepEdits,
}

pub struct Prepared {
    base: String,
    commit: String,
    message: String,
}

impl Stage {
    pub fn take_checkout(&self, admin: &User) -> Result<()> {
        let git = |args: &[&str]| in_checkout(admin, args);
        if git(&["branch", "--show-current"])?.trim() != "main" {
            bail!("{DIR} isn't on main: switch to it first");
        }
        if git(&["rev-parse", "HEAD"])?.trim() != self.base {
            bail!(
                "main in {DIR} isn't at {}, the commit this machine checked",
                short(&self.base)
            );
        }
        let index = tempfile::TempDir::new()?;
        admin.own(index.path())?;
        let index = format!("GIT_INDEX_FILE={}", index.path().join("index").display());
        let indexed = |args: &[&str]| {
            let args = [&[index.as_str(), "git", "-C", DIR][..], args].concat();
            cmd::bytes(As::User(admin), "env", &args)
        };
        indexed(&["read-tree", "HEAD"])?;
        indexed(&["add", "--all"])?;
        let patch = indexed(&["diff", "--cached", "--binary", "HEAD"])?;
        if patch.is_empty() {
            return Ok(());
        }
        let mut apply = Git::at(GIT_DIR).command(&["apply", "--whitespace=nowarn", "-"]);
        apply.current_dir(&self.dir).env("GIT_WORK_TREE", &self.dir);
        cmd::output_with_input(apply, "git", &["apply"], &patch).map(drop)
    }

    fn staged<S: AsRef<OsStr>>(&self, args: &[S]) -> Result<String> {
        let mut command = Git::at(GIT_DIR).command(args);
        command
            .current_dir(&self.dir)
            .env("GIT_INDEX_FILE", self.index.path().join("index"))
            .env("GIT_WORK_TREE", &self.dir);
        let out = cmd::output(command, "git", args)?;
        Ok(String::from_utf8(out)?.trim_end().to_owned())
    }

    fn tree(&self) -> Result<String> {
        self.staged(&["add", "--all", "--force"])?;
        self.staged(&["write-tree"])
    }

    pub fn machine_signer(&self) -> Result<Option<Signer>> {
        let Some(key) = sign::machine_key()? else {
            return Ok(None);
        };
        let signers = Git::at(GIT_DIR).signers(&self.accepted)?;
        Ok(signers.into_iter().find(|s| &s.key == key.public_key()))
    }

    pub fn keep_if_signable(&mut self, signer: Option<&Signer>) -> Result<Option<String>> {
        let tree = self.tree()?;
        if tree == self.kept {
            return Ok(None);
        }
        let git = Git::at(GIT_DIR);
        let changed = git.changed(&self.accepted, &tree)?;
        let why = match signer {
            None => Some("this machine signs nothing for the fleet".to_owned()),
            Some(signer) => {
                let before = git.machines(&self.accepted)?;
                sign::allowed(signer, &changed, &before, &git.machines(&tree)?)
                    .err()
                    .map(|e| format!("{e:#}"))
            }
        };
        let Some(why) = why else {
            self.kept = tree;
            return Ok(None);
        };
        let undone = git.changed(&self.kept, &tree)?;
        let source = format!("--source={}", self.kept);
        let restore = [
            "--literal-pathspecs",
            "restore",
            &source,
            "--worktree",
            "--",
        ];
        let args: Vec<OsString> = restore
            .iter()
            .map(OsString::from)
            .chain(undone.into_iter().map(PathBuf::into_os_string))
            .collect();
        self.staged(&args)?;
        self.tree()?;
        Ok(Some(why))
    }

    pub fn prepare(&self, who: &str, message: &str, key: &PrivateKey) -> Result<Option<Prepared>> {
        let git = Git::at(GIT_DIR);
        let tree = self.tree()?;
        if self.base == self.accepted
            && Some(&tree) == git.rev(&format!("{}^{{tree}}", self.base))?.as_ref()
        {
            return Ok(None);
        }
        super::check_loads(&self.dir)
            .context("machines couldn't load the fleet after this change, so nothing changed")?;
        let commit =
            verify::signed_commit(&git, &tree, &self.base, &self.accepted, who, message, key)?;
        Ok(Some(Prepared {
            base: self.base.clone(),
            commit,
            message: message.to_owned(),
        }))
    }

    pub fn commit(
        self,
        admin: &User,
        who: &str,
        message: &str,
        key: &PrivateKey,
        prompt: &mut Prompt,
        advance: Advance,
    ) -> Result<Outcome> {
        let prepared = self.prepare(who, message, key)?;
        let base = self.base.clone();
        self.finish()?;
        let Some(prepared) = prepared else {
            return Ok(Outcome::Unchanged(base));
        };
        prepared.show()?;
        if let Advance::KeepEdits = advance {
            println!("These are the edits in {DIR}: read them there in a git app.");
        }
        if !prompt.confirm("Commit and push")? {
            println!("Nothing changed.");
            return Ok(Outcome::Declined);
        }
        prepared.publish(admin, advance).map(Outcome::Pushed)
    }

    pub fn finish(self) -> Result<()> {
        fs::remove_dir_all(&self.dir).with_context(|| format!("removing {STAGING}"))
    }
}

impl Prepared {
    pub fn show(&self) -> Result<()> {
        println!("{}:", self.message);
        print_files(&Git::at(GIT_DIR), &self.base, &self.commit)
    }

    pub fn publish(self, admin: &User, advance: Advance) -> Result<String> {
        let git = Git::at(GIT_DIR);
        git.out(&["update-ref", STAGED, &self.commit])?;
        let bundle = tempfile::Builder::new().suffix(".bundle").tempfile()?;
        let path = bundle.path().to_string_lossy().into_owned();
        git.out(&[
            "bundle",
            "create",
            "--quiet",
            &path,
            STAGED,
            &format!("^{}", self.base),
        ])?;
        fs::set_permissions(bundle.path(), fs::Permissions::from_mode(0o644))?;
        publish(admin, &self.base, &self.commit, bundle.path(), advance)?;
        println!("Committed {} {}", short(&self.commit), self.message);
        if let Some(refused) = sync::sync(admin, Remote::Skip)?.refused {
            println!(
                "This machine refuses {}: {}",
                short(&refused.commit),
                refused.why
            );
        }
        Ok(self.commit)
    }
}

fn publish(admin: &User, base: &str, commit: &str, bundle: &Path, advance: Advance) -> Result<()> {
    let git = |args: &[&str]| in_checkout(admin, args);
    let main = git(&["rev-parse", "--verify", "main"])?;
    if main.trim() != base {
        return Err(moved());
    }
    git(&["fetch", "--quiet", &bundle.to_string_lossy(), STAGED])?;
    match advance {
        Advance::Merge => move_main(admin, base, commit)?,
        Advance::KeepEdits => git(&["reset", "--quiet", commit]).map(drop)?,
    }
    if let Err(e) = git(&["push", "--quiet", "origin", "main"]) {
        println!("Couldn't push the fleet, so push it yourself: {e:#}");
    }
    Ok(())
}
