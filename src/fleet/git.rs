use super::{
    MANIFEST, Manifest,
    machines::{MACHINES, Machines},
};
use crate::{
    trust::sign::{Commit, Signer},
    util::cmd,
};
use anyhow::{Context, Result};
use std::{ffi::OsStr, os::unix::ffi::OsStrExt, path::PathBuf, process::Command};

pub const GIT_DIR: &str = "/var/lib/mothership/fleet.git";

pub struct Git {
    dir: PathBuf,
    env: Vec<(String, String)>,
}

impl Git {
    pub fn at(dir: impl Into<PathBuf>) -> Git {
        Git {
            dir: dir.into(),
            env: Vec::new(),
        }
    }

    pub fn with_env(mut self, key: &str, value: String) -> Git {
        self.env.push((key.to_owned(), value));
        self
    }

    pub fn command<S: AsRef<OsStr>>(&self, args: &[S]) -> Command {
        let mut git = Command::new("git");
        git.arg("--git-dir")
            .arg(&self.dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .envs(self.env.iter().map(|(k, v)| (k, v)));
        git
    }

    pub fn out<S: AsRef<OsStr>>(&self, args: &[S]) -> Result<String> {
        let bytes = cmd::output(self.command(args), "git", args)?;
        Ok(String::from_utf8(bytes)?.trim_end().to_owned())
    }

    pub fn with_input(&self, args: &[&str], input: &str) -> Result<String> {
        let out = cmd::output_with_input(self.command(args), "git", args, input.as_bytes())?;
        Ok(String::from_utf8(out)?.trim_end().to_owned())
    }

    fn succeeds(&self, args: &[&str]) -> Result<bool> {
        let status = self
            .command(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .context("running git")?;
        Ok(status.success())
    }

    pub fn rev(&self, name: &str) -> Result<Option<String>> {
        let args = ["rev-parse", "--verify", "--quiet", "--end-of-options", name];
        Ok(match self.succeeds(&args)? {
            true => Some(self.out(&args)?),
            false => None,
        })
    }

    pub fn is_ancestor(&self, older: &str, newer: &str) -> Result<bool> {
        self.succeeds(&["merge-base", "--is-ancestor", older, newer])
    }

    pub fn blob(&self, commit: &str, path: &str) -> Result<Option<String>> {
        let name = format!("{commit}:{path}");
        if !self.succeeds(&["cat-file", "-e", &name])? {
            return Ok(None);
        }
        self.out(&["cat-file", "blob", &name]).map(Some)
    }

    pub fn files(&self, commit: &str, dir: &str) -> Result<Vec<PathBuf>> {
        self.paths(&["ls-tree", "-r", "-z", "--name-only", commit, "--", dir])
    }

    pub fn changed(&self, from: &str, to: &str) -> Result<Vec<PathBuf>> {
        self.paths(&[
            "diff-tree",
            "-r",
            "-z",
            "--name-only",
            "--no-commit-id",
            from,
            to,
        ])
    }

    fn paths(&self, args: &[&str]) -> Result<Vec<PathBuf>> {
        Ok(cmd::output(self.command(args), "git", args)?
            .split(|&byte| byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| PathBuf::from(OsStr::from_bytes(path)))
            .collect())
    }

    pub fn machines(&self, commit: &str) -> Result<Machines> {
        let mut files = Vec::new();
        for rel in self.files(commit, MACHINES)? {
            let text = self.out(&["cat-file", "blob", &format!("{commit}:{}", rel.display())])?;
            files.push((rel, text));
        }
        Machines::parse(files).with_context(|| format!("reading {MACHINES}/ in {}", short(commit)))
    }

    pub fn commit(&self, commit: &str) -> Result<Commit> {
        let args = ["cat-file", "commit", commit];
        Commit::parse(&cmd::output(self.command(&args), "git", &args)?)
    }

    pub fn signers(&self, commit: &str) -> Result<Vec<Signer>> {
        self.machines(commit)?.signers()
    }

    pub fn recovery(&self, commit: &str) -> Result<Option<Signer>> {
        let Some(text) = self.blob(commit, MANIFEST)? else {
            return Ok(None);
        };
        let manifest: Manifest = toml::from_str(&text)
            .with_context(|| format!("reading {MANIFEST} in {}", short(commit)))?;
        manifest.recovery_signer()
    }
}

pub fn short(commit: &str) -> &str {
    &commit[..commit.len().min(12)]
}
