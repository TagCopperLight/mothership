use super::{DIR, MANIFEST, Manifest};
use crate::{
    cmd::{self, As},
    file,
    sign::{self, Commit, SIGNERS, Signer},
    update::BINARY,
    user::User,
};
use anyhow::{Context, Result, bail};
use ssh_key::PrivateKey;
use std::{
    ffi::OsStr,
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

pub const GIT_DIR: &str = "/var/lib/mothership/fleet.git";
pub const CURRENT: &str = "/var/lib/mothership/current";
const EXPORTS: &str = "/var/lib/mothership/checked";
const CHECKED: &str = "refs/mothership/checked";
const MAIN: &str = "refs/incoming/heads/main";
const ORIGIN: &str = "refs/incoming/origin/main";
const STAGED: &str = "refs/mothership/staged";
const STAGING: &str = "/var/lib/mothership/staging";

pub struct Git {
    dir: PathBuf,
}

impl Git {
    pub fn at(dir: impl Into<PathBuf>) -> Git {
        Git { dir: dir.into() }
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
            .env_remove("GIT_INDEX_FILE");
        git
    }

    pub fn out<S: AsRef<OsStr>>(&self, args: &[S]) -> Result<String> {
        let bytes = cmd::output(self.command(args), "git", args)?;
        Ok(String::from_utf8(bytes)?.trim_end().to_owned())
    }

    fn with_input(&self, args: &[&str], input: &str) -> Result<String> {
        let mut child = self
            .command(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("running git")?;
        child
            .stdin
            .take()
            .expect("piped above")
            .write_all(input.as_bytes())?;
        let out = child.wait_with_output()?;
        if !out.status.success() {
            bail!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8(out.stdout)?.trim_end().to_owned())
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

    fn is_ancestor(&self, older: &str, newer: &str) -> Result<bool> {
        self.succeeds(&["merge-base", "--is-ancestor", older, newer])
    }

    fn blob(&self, commit: &str, path: &str) -> Result<Option<String>> {
        let name = format!("{commit}:{path}");
        if !self.succeeds(&["cat-file", "-e", &name])? {
            return Ok(None);
        }
        self.out(&["cat-file", "blob", &name]).map(Some)
    }

    fn signers(&self, commit: &str) -> Result<Vec<Signer>> {
        match self.blob(commit, SIGNERS)? {
            Some(text) => sign::parse_signers(&text)
                .with_context(|| format!("reading {SIGNERS} in {}", short(commit))),
            None => Ok(Vec::new()),
        }
    }

    fn recovery(&self, commit: &str) -> Result<Option<Signer>> {
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

#[derive(Debug)]
pub struct Refused {
    pub commit: String,
    pub author: String,
    pub subject: String,
    pub why: String,
}

pub struct Checked {
    pub commit: String,
    pub refused: Option<Refused>,
}

pub fn check(git: &Git, from: &str, to: &str) -> Result<Checked> {
    if !git.is_ancestor(from, to)? {
        bail!(
            "main doesn't follow {}, the last commit this machine checked: rebase it onto that \
             commit",
            short(from)
        );
    }
    let list = git.out(&[
        "rev-list",
        "--first-parent",
        "--reverse",
        &format!("{from}..{to}"),
    ])?;
    let mut good = from.to_owned();
    for commit in list.lines() {
        if let Err(why) = verify(git, &good, commit) {
            let line = git.out(&["log", "-1", "--format=%an%x00%s", commit])?;
            let (author, subject) = line.split_once('\0').unwrap_or((&line, ""));
            return Ok(Checked {
                commit: good,
                refused: Some(Refused {
                    commit: commit.to_owned(),
                    author: author.to_owned(),
                    subject: subject.to_owned(),
                    why: format!("{why:#}"),
                }),
            });
        }
        good = commit.to_owned();
    }
    Ok(Checked {
        commit: good,
        refused: None,
    })
}

fn verify(git: &Git, parent: &str, commit: &str) -> Result<String> {
    let first = git.out(&["rev-parse", &format!("{commit}^1")])?;
    if first != parent {
        bail!("it doesn't follow {} on main's line", short(parent));
    }
    let before = git.signers(parent)?;
    let mut trusted = before.clone();
    trusted.extend(git.recovery(parent)?);
    if trusted.is_empty() {
        return Ok("nobody, as the fleet named no signer".to_owned());
    }
    let args = ["cat-file", "commit", commit];
    let signed = Commit::parse(&cmd::output(git.command(&args), "git", &args)?)?;
    let Some(signer) = signed.signer(&trusted) else {
        bail!(match signed.signature {
            None => "nobody signed it",
            Some(_) => "a key the fleet doesn't list signed it",
        });
    };
    let changed = git.out(&[
        "diff-tree",
        "-r",
        "--name-only",
        "--no-commit-id",
        parent,
        commit,
    ])?;
    let changed: Vec<PathBuf> = changed.lines().map(PathBuf::from).collect();
    sign::allowed(signer, &changed, &before, &git.signers(commit)?)?;
    Ok(signer.name.clone())
}

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
    Tip,
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
    let anchor = match (anchor, git.rev(CHECKED)?) {
        (Anchor::Kept, Some(checked)) => checked,
        (Anchor::Kept, None) => {
            println!(
                "This machine trusts the fleet at {} from now on.",
                short(&tip)
            );
            tip.clone()
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
        (Anchor::Tip, _) => tip.clone(),
    };
    let checked = check(&git, &anchor, &tip)?;
    git.out(&["update-ref", CHECKED, &checked.commit])?;
    export(&git, &checked.commit)?;
    if checked.commit != main
        && git.is_ancestor(&main, &checked.commit)?
        && let Err(e) = advance_main(admin, &main, &checked.commit)
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

fn fetch(git: &Git, admin: &User, remote: Remote) -> Result<(String, String)> {
    if !git.dir.exists() {
        cmd::root("git", &["init", "--quiet", "--bare", GIT_DIR])?;
        git.out(&["config", "fetch.fsckObjects", "true"])?;
    }
    sign_through_mothership(admin)?;
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

fn sign_through_mothership(admin: &User) -> Result<()> {
    let Some(key) = sign::machine_key()? else {
        return Ok(());
    };
    let wanted = [
        ("gpg.format", "ssh".to_owned()),
        ("gpg.ssh.program", BINARY.to_owned()),
        ("user.signingkey", format!("key::{}", sign::public(&key)?)),
    ];
    let config = |args: &[&str]| in_checkout(admin, &[&["config", "--local"][..], args].concat());
    let listed = config(&["--list"])?;
    for (name, value) in wanted {
        if !listed.lines().any(|line| line == format!("{name}={value}")) {
            config(&[name, &value])?;
        }
    }
    Ok(())
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
            Some(parent) => match verify(&git, &parent, commit) {
                Ok(signer) => format!("signed by {signer}"),
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
    file::link(Path::new(CURRENT), &done)?;
    for entry in fs::read_dir(exports)? {
        let path = entry?.path();
        if path != done {
            fs::remove_dir_all(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }
    Ok(())
}

fn extract(git: &Git, commit: &str, done: &Path) -> Result<()> {
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

pub struct Stage {
    pub dir: PathBuf,
    base: String,
}

pub fn stage() -> Result<Stage> {
    let git = Git::at(GIT_DIR);
    let base = git
        .rev(CHECKED)?
        .context("this machine has checked no commit of the fleet yet")?;
    let dir = PathBuf::from(STAGING);
    if dir.exists() {
        fs::remove_dir_all(&dir).with_context(|| format!("removing {STAGING}"))?;
    }
    extract(&git, &base, &dir)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    Ok(Stage { dir, base })
}

impl Stage {
    pub fn commit(self, admin: &User, message: &str, key: &PrivateKey) -> Result<Option<String>> {
        let git = Git::at(GIT_DIR);
        let index = tempfile::TempDir::new()?;
        let staged = |args: &[&str]| -> Result<String> {
            let mut command = git.command(args);
            command
                .env("GIT_INDEX_FILE", index.path().join("index"))
                .env("GIT_WORK_TREE", &self.dir);
            let out = cmd::output(command, "git", args)?;
            Ok(String::from_utf8(out)?.trim_end().to_owned())
        };
        staged(&["read-tree", &self.base])?;
        staged(&["add", "--all", "--force"])?;
        let tree = staged(&["write-tree"])?;
        fs::remove_dir_all(&self.dir).with_context(|| format!("removing {STAGING}"))?;
        if Some(&tree) == git.rev(&format!("{}^{{tree}}", self.base))?.as_ref() {
            return Ok(None);
        }
        let config = |name: &str| -> Result<String> {
            author(admin, name)?.with_context(|| {
                format!(
                    "{} has no git {name}: set it with `git -C {DIR} config {name} …`",
                    admin.name
                )
            })
        };
        let who = format!(
            "{} <{}> {} +0000",
            config("user.name")?,
            config("user.email")?,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs()
        );
        let payload = format!(
            "tree {tree}\nparent {}\nauthor {who}\ncommitter {who}\n\n{message}\n",
            self.base
        );
        let signature = sign::sign(key, payload.as_bytes())?;
        let object = payload.replacen(
            "\n\n",
            &format!("\ngpgsig {}\n\n", signature.trim_end().replace('\n', "\n ")),
            1,
        );
        let commit = git.with_input(&["hash-object", "-t", "commit", "-w", "--stdin"], &object)?;
        if let Err(why) = verify(&git, &self.base, &commit) {
            bail!("the fleet's machines would refuse this commit, so nothing changed: {why:#}");
        }
        git.out(&["update-ref", STAGED, &commit])?;
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
        publish(admin, &self.base, &commit, bundle.path())?;
        println!("Committed {} {message}", short(&commit));
        if let Some(refused) = sync(admin, Remote::Skip)?.refused {
            println!(
                "This machine refuses {}: {}",
                short(&refused.commit),
                refused.why
            );
        }
        Ok(Some(commit))
    }
}

fn publish(admin: &User, base: &str, commit: &str, bundle: &Path) -> Result<()> {
    let git = |args: &[&str]| in_checkout(admin, args);
    let main = git(&["rev-parse", "--verify", "main"])?;
    if main.trim() != base {
        bail!("main moved in {DIR} while mothership worked: run it again");
    }
    git(&["fetch", "--quiet", &bundle.to_string_lossy(), STAGED])?;
    advance_main(admin, base, commit)?;
    if let Err(e) = git(&["push", "--quiet", "origin", "main"]) {
        println!("Couldn't push the fleet, so push it yourself: {e:#}");
    }
    Ok(())
}

fn in_checkout(admin: &User, args: &[&str]) -> Result<String> {
    cmd::stdout(As::User(admin), "git", &[&["-C", DIR][..], args].concat())
}

pub fn author(admin: &User, name: &str) -> Result<Option<String>> {
    let args = ["-C", DIR, "config", name];
    let value = cmd::bytes_allowing_exit(As::User(admin), "git", &args, 1)?;
    let value = String::from_utf8(value)?.trim().to_owned();
    Ok((!value.is_empty()).then_some(value))
}

pub fn set_author(admin: &User, name: &str, value: &str) -> Result<()> {
    in_checkout(admin, &["config", name, value]).map(drop)
}

pub fn origin(admin: &User) -> Result<String> {
    Ok(in_checkout(admin, &["remote", "get-url", "origin"])?
        .trim()
        .to_owned())
}

fn advance_main(admin: &User, from: &str, to: &str) -> Result<()> {
    let git = |args: &[&str]| in_checkout(admin, args);
    match git(&["branch", "--show-current"])?.trim() {
        "main" => git(&["merge", "--ff-only", "--quiet", to]),
        _ => git(&["update-ref", "refs/heads/main", to, from]),
    }
    .map(drop)
}

#[cfg(test)]
#[path = "../../tests/checked.rs"]
mod tests;
