use super::{
    DIR, MANIFEST, Manifest,
    homes::{self, Reader},
    machines::{MACHINES, Machines},
};
use crate::{
    secrets,
    services::{self, confirm::Unconfirmed},
    system::user::User,
    trust::github,
    trust::sign::{self, Commit, Reach, Signer},
    util::cmd::{self, As},
    util::file,
    util::prompt::Prompt,
    util::style,
};
use anyhow::{Context, Result, bail};
use ssh_key::PrivateKey;
use std::{
    ffi::OsStr,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

pub const GIT_DIR: &str = "/var/lib/mothership/fleet.git";
pub const CURRENT: &str = "/var/lib/mothership/current";
const EXPORTS: &str = "/var/lib/mothership/checked";
const LISTS: &str = "/var/lib/mothership/recipients";
const CHECKED: &str = "refs/mothership/checked";
const MAIN: &str = "refs/incoming/heads/main";
const ORIGIN: &str = "refs/incoming/origin/main";
const STAGED: &str = "refs/mothership/staged";
const STAGING: &str = "/var/lib/mothership/staging";
const PLAIN: &str = "100644";

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

    fn machines(&self, commit: &str) -> Result<Machines> {
        let listed = self.out(&["ls-tree", "--name-only", commit, &format!("{MACHINES}/")])?;
        let mut files = Vec::new();
        for rel in listed.lines() {
            let text = self.out(&["cat-file", "blob", &format!("{commit}:{rel}")])?;
            files.push((PathBuf::from(rel), text));
        }
        Machines::parse(files).with_context(|| format!("reading {MACHINES}/ in {}", short(commit)))
    }

    fn signers(&self, commit: &str) -> Result<Vec<Signer>> {
        self.machines(commit)?.signers()
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
    pub date: String,
    pub subject: String,
    pub why: String,
}

pub struct Checked {
    pub commit: String,
    pub refused: Option<Refused>,
    pub steps: Vec<Step>,
}

pub struct Step {
    pub parent: String,
    pub commit: String,
    pub reach: Reach,
}

pub fn check(git: &Git, from: &str, to: &str) -> Result<Checked> {
    follows(git, from, to)?;
    let list = git.out(&[
        "rev-list",
        "--first-parent",
        "--reverse",
        &format!("{from}..{to}"),
    ])?;
    let mut good = from.to_owned();
    let mut refused = None;
    let mut steps = Vec::new();
    for commit in list.lines() {
        match admit(git, &good, commit, refused.as_ref()) {
            Ok(signer) => {
                steps.push(Step {
                    parent: good,
                    commit: commit.to_owned(),
                    reach: signer.map_or(Reach::Everything, |signer| signer.reach),
                });
                good = commit.to_owned();
                refused = None;
            }
            Err(why) if refused.is_none() => refused = Some(refused_at(git, commit, &why)?),
            Err(_) => {}
        }
    }
    Ok(Checked {
        commit: good,
        refused,
        steps,
    })
}

fn follows(git: &Git, from: &str, to: &str) -> Result<()> {
    if !git.is_ancestor(from, to)? {
        bail!(
            "main doesn't follow {}, the last commit this machine checked: rebase it onto that \
             commit",
            short(from)
        );
    }
    Ok(())
}

fn refused_at(git: &Git, commit: &str, why: &anyhow::Error) -> Result<Refused> {
    let line = git.out(&["log", "-1", "--format=%an%x00%as%x00%s", commit])?;
    let [author, date, subject] = line.splitn(3, '\0').collect::<Vec<_>>()[..] else {
        bail!("git log printed {line}");
    };
    Ok(Refused {
        commit: commit.to_owned(),
        author: style::clean(author),
        date: style::clean(date),
        subject: style::clean(subject),
        why: format!("{why:#}"),
    })
}

fn admit(git: &Git, good: &str, commit: &str, refused: Option<&Refused>) -> Result<Option<Signer>> {
    let Some(refused) = refused else {
        return verify(git, good, commit);
    };
    if git.rev(&format!("{}^1", refused.commit))?.as_deref() != Some(good) {
        bail!(
            "it can't vouch for {}, which doesn't follow {} on main's line",
            short(&refused.commit),
            short(good)
        );
    }
    let signer = approve(git, good, commit)?;
    if let Some(signer) = &signer
        && signer.reach == Reach::Versions
    {
        bail!(
            "{} may change only versions, so it can't vouch for the commits before it",
            signer.name
        );
    }
    Ok(signer)
}

pub fn verify(git: &Git, parent: &str, commit: &str) -> Result<Option<Signer>> {
    let first = git.out(&["rev-parse", &format!("{commit}^1")])?;
    if first != parent {
        bail!("it doesn't follow {} on main's line", short(parent));
    }
    approve(git, parent, commit)
}

fn approve(git: &Git, base: &str, commit: &str) -> Result<Option<Signer>> {
    let after = git.machines(commit)?;
    sign::distinct(&after.signers()?, git.recovery(commit)?.as_ref())?;
    let before = git.machines(base)?;
    let mut trusted = before.signers()?;
    trusted.extend(git.recovery(base)?);
    if trusted.is_empty() {
        return Ok(None);
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
        base,
        commit,
    ])?;
    let changed: Vec<PathBuf> = changed.lines().map(PathBuf::from).collect();
    sign::allowed(signer, &changed, &before, &after)?;
    if signer.reach == Reach::Versions {
        for path in &changed {
            let text = path.to_string_lossy();
            let (Some(old), Some(new)) = (git.blob(base, &text)?, git.blob(commit, &text)?) else {
                bail!("{} can't add or remove {text}", signer.name);
            };
            for side in [base, commit] {
                if git.out(&["ls-tree", "--format=%(objectmode)", side, "--", &text])? != PLAIN {
                    bail!(
                        "{} can change only a plain file's text, not {text}'s kind",
                        signer.name
                    );
                }
            }
            services::versions_only(&old, &new)
                .with_context(|| format!("{} can't sign this change to {text}", signer.name))?;
        }
    }
    Ok(Some(signer.clone()))
}

fn note_inherited(git: &Git, anchor: &str) -> Result<()> {
    let mut unconfirmed = Unconfirmed::load()?;
    inherited(git, anchor, &mut unconfirmed)?;
    unconfirmed.save()
}

fn inherited(git: &Git, anchor: &str, unconfirmed: &mut Unconfirmed) -> Result<()> {
    for path in git
        .out(&["ls-tree", "-r", "--name-only", anchor, "--", super::TYPES])?
        .lines()
    {
        let Some(service) = services::service_of(Path::new(path)) else {
            continue;
        };
        let mut open = services::changed_pins(None, git.blob(anchor, path)?.as_deref());
        let history = git.out(&["rev-list", "--first-parent", anchor, "--", path])?;
        for commit in history.lines() {
            if open.is_empty() {
                break;
            }
            let parent = git.rev(&format!("{commit}^1"))?;
            let old = match &parent {
                Some(parent) => git.blob(parent, path)?,
                None => None,
            };
            let set = services::changed_pins(old.as_deref(), git.blob(commit, path)?.as_deref());
            let by_versions = match &parent {
                Some(parent) => !matches!(
                    verify(git, parent, commit),
                    Ok(Some(Signer {
                        reach: Reach::Everything | Reach::Folders(_),
                        ..
                    })) | Ok(None)
                ),
                None => false,
            };
            let (now, later): (Vec<_>, Vec<_>) =
                open.into_iter().partition(|pin| set.contains(pin));
            unconfirmed.note(&service, now, by_versions);
            open = later;
        }
        unconfirmed.note(&service, open, true);
    }
    Ok(())
}

fn note_unconfirmed(git: &Git, steps: &[Step]) -> Result<()> {
    if steps.is_empty() {
        return Ok(());
    }
    let mut unconfirmed = Unconfirmed::load()?;
    for step in steps {
        let changed = git.out(&[
            "diff-tree",
            "-r",
            "--name-only",
            "--no-commit-id",
            &step.parent,
            &step.commit,
        ])?;
        for path in changed.lines() {
            let Some(service) = services::service_of(Path::new(path)) else {
                continue;
            };
            let pins = services::changed_pins(
                git.blob(&step.parent, path)?.as_deref(),
                git.blob(&step.commit, path)?.as_deref(),
            );
            unconfirmed.note(&service, pins, step.reach == Reach::Versions);
        }
    }
    unconfirmed.save()
}

pub fn signed_commit(
    git: &Git,
    tree: &str,
    parent: &str,
    accepted: &str,
    who: &str,
    message: &str,
    key: &PrivateKey,
) -> Result<String> {
    let who = format!(
        "{who} {} +0000",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs()
    );
    let payload =
        format!("tree {tree}\nparent {parent}\nauthor {who}\ncommitter {who}\n\n{message}\n");
    let signature = sign::sign(key, payload.as_bytes())?;
    let object = payload.replacen(
        "\n\n",
        &format!("\ngpgsig {}\n\n", signature.trim_end().replace('\n', "\n ")),
        1,
    );
    let commit = git.with_input(&["hash-object", "-t", "commit", "-w", "--stdin"], &object)?;
    let before = check(git, accepted, parent)?;
    if let Err(why) = admit(git, &before.commit, &commit, before.refused.as_ref()) {
        bail!("the fleet's machines would refuse this commit, so nothing changed: {why:#}");
    }
    Ok(commit)
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
        (Anchor::Tip, _) => tip.clone(),
    };
    if fresh {
        note_inherited(&git, &anchor)?;
    }
    let checked = check(&git, &anchor, &tip)?;
    note_unconfirmed(&git, &checked.steps)?;
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

fn fetch(git: &Git, admin: &User, remote: Remote) -> Result<(String, String)> {
    if !git.dir.exists() {
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

fn tips(git: &Git) -> Result<(String, String)> {
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
            Some(parent) => match verify(&git, &parent, commit) {
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

pub fn linked_when_written(dir: &Path, file: &Path) -> Result<Option<Vec<Reader>>> {
    if dir != Path::new(STAGING) {
        return Ok(None);
    }
    let git = Git::at(GIT_DIR);
    let base = git
        .rev(CHECKED)?
        .context("this machine has checked no commit")?;
    let rel = file.strip_prefix(dir)?;
    let rel_text = rel
        .to_str()
        .context("a shared file whose name isn't UTF-8")?;
    let written = git.out(&["log", "-1", "--format=%H", &base, "--", rel_text])?;
    if written.is_empty() {
        return Ok(None);
    }
    let listing = git.out(&["ls-tree", "-r", "-z", &written, "--", super::TYPES])?;
    let mut readers = Vec::new();
    for entry in listing.split('\0').filter(|entry| !entry.is_empty()) {
        let Some((meta, path)) = entry.split_once('\t') else {
            bail!("git ls-tree printed {entry}");
        };
        let [mode, _, blob] = meta.split(' ').collect::<Vec<_>>()[..] else {
            bail!("git ls-tree printed {entry}");
        };
        let path = Path::new(path);
        let reader = match mode {
            "120000" => homes::owner(Path::new(""), path).ok(),
            _ => None,
        };
        let Some(reader) = reader else {
            continue;
        };
        let target = git.out(&["cat-file", "blob", blob])?;
        let mut resolved = PathBuf::new();
        for part in path.parent().unwrap_or(path).join(target).components() {
            match part {
                std::path::Component::ParentDir => drop(resolved.pop()),
                std::path::Component::Normal(name) => resolved.push(name),
                _ => {}
            }
        }
        if resolved == rel {
            readers.push(reader);
        }
    }
    Ok(Some(readers))
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
    write_lists(&done)?;
    file::link(Path::new(CURRENT), &done)?;
    for entry in fs::read_dir(exports)? {
        let path = entry?.path();
        if path != done {
            fs::remove_dir_all(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }
    Ok(())
}

pub fn list_path(type_name: &str, user: &str) -> PathBuf {
    Path::new(LISTS).join(type_name).join(format!("{user}.txt"))
}

fn write_lists(tree: &Path) -> Result<()> {
    let machines = Machines::load(tree)?;
    let recovery = Manifest::read(tree)?.recovery();
    let partial = Path::new(LISTS).with_file_name(".recipients");
    if partial.exists() {
        fs::remove_dir_all(&partial)?;
    }
    for reader in homes::readers(tree)? {
        let Reader::Home(home) = &reader else {
            continue;
        };
        let keys: Vec<String> = machines
            .sealed(std::slice::from_ref(&reader), recovery.as_ref())
            .into_iter()
            .map(|recipient| recipient.key)
            .collect();
        let path = partial
            .join(&home.type_name)
            .join(format!("{}.txt", home.user));
        file::write(&path, secrets::list_text(&keys).as_bytes(), 0o644)?;
    }
    if Path::new(LISTS).exists() {
        fs::remove_dir_all(LISTS)?;
    }
    if partial.exists() {
        fs::rename(&partial, LISTS)?;
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
    let (main, tip) = tips(&git)?;
    follows(&git, &accepted, &tip)?;
    let base = match tip == accepted {
        true => accepted.clone(),
        false => match settle(&git, &accepted, &tip, prompt)? {
            Settle::Nothing => {
                println!("Nothing changed.");
                return Ok(None);
            }
            Settle::Vouch => {
                if main != tip {
                    move_main(&super::admin()?, &main, &tip)?;
                }
                tip
            }
            Settle::Drop => {
                drop_after(&git, &super::admin()?, &accepted, &main)?;
                accepted.clone()
            }
        },
    };
    let dir = PathBuf::from(STAGING);
    if dir.exists() {
        fs::remove_dir_all(&dir).with_context(|| format!("removing {STAGING}"))?;
    }
    extract(&git, &base, &dir)?;
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
    if let Some(repo) = github::repo(&origin(&super::admin()?)?) {
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

    fn staged(&self, args: &[&str]) -> Result<String> {
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
        let changed = self.staged(&["diff-tree", "-r", "--name-only", &self.accepted, &tree])?;
        let changed: Vec<PathBuf> = changed.lines().map(PathBuf::from).collect();
        let why = match signer {
            None => Some("this machine signs nothing for the fleet".to_owned()),
            Some(signer) => {
                let git = Git::at(GIT_DIR);
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
        let undone = self.staged(&["diff-tree", "-r", "--name-only", &self.kept, &tree])?;
        let paths: Vec<&str> = undone.lines().collect();
        let source = format!("--source={}", self.kept);
        self.staged(&[&["restore", &source, "--worktree", "--"][..], &paths].concat())?;
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
        let commit = signed_commit(&git, &tree, &self.base, &self.accepted, who, message, key)?;
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
        if let Some(refused) = sync(admin, Remote::Skip)?.refused {
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

fn in_checkout(admin: &User, args: &[&str]) -> Result<String> {
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

fn moved() -> anyhow::Error {
    anyhow::anyhow!("main moved in {DIR} while mothership worked: run it again")
}

fn move_main(admin: &User, from: &str, to: &str) -> Result<()> {
    let git = |args: &[&str]| in_checkout(admin, args);
    match git(&["branch", "--show-current"])?.trim() {
        "main" if git(&["rev-parse", "HEAD"])?.trim() != from => Err(moved()),
        "main" => git(&["update-index", "-q", "--refresh"])
            .and_then(|_| git(&["reset", "--keep", "--quiet", to])),
        _ => git(&["update-ref", "refs/heads/main", to, from]),
    }
    .map(drop)
}

#[cfg(test)]
#[path = "../../tests/fleet/checked.rs"]
mod tests;
