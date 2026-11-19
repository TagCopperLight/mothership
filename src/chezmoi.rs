use crate::{
    cmd::{self, As, Cmd},
    fleet::{ChezmoiConfig, Fleet, Member},
    secrets,
    snapshot::Snapshot,
    user::{self, User},
};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;

pub fn managed(user: &User, include: &str) -> Result<Vec<PathBuf>> {
    let args = ["managed", "--include", include, "--path-style", "absolute"];
    cmd::stdout(As::User(user), "chezmoi", &args)?
        .lines()
        .map(|line| in_home(user, Path::new(line)))
        .collect()
}

pub fn in_home(user: &User, path: &Path) -> Result<PathBuf> {
    let plain = path
        .strip_prefix(&user.home)
        .is_ok_and(|rel| rel.components().all(|c| matches!(c, Component::Normal(_))));
    if !plain {
        bail!(
            "chezmoi named {}, which isn't a plain path in {}",
            path.display(),
            user.home.display()
        );
    }
    Ok(path.to_path_buf())
}

#[derive(Clone)]
pub struct Root {
    user: User,
    config: ChezmoiConfig,
}

impl Root {
    pub fn new(fleet: &Fleet, member: &Member) -> Root {
        Root {
            user: member.user().clone(),
            config: member.chezmoi_config(fleet),
        }
    }

    pub fn user(&self) -> &User {
        &self.user
    }

    pub fn source_path(&self, target: &Path) -> Result<PathBuf> {
        let empty = Snapshot::take(&self.user, &[])?;
        let target = target.to_string_lossy();
        let out = self.output(&empty, &["source-path", &target])?;
        let source = PathBuf::from(String::from_utf8(out)?.trim());
        let inside = source
            .strip_prefix(&self.config.source)
            .is_ok_and(|rel| rel.components().all(|c| matches!(c, Component::Normal(_))));
        if !inside {
            bail!(
                "chezmoi put {target} outside {}",
                self.config.source.display()
            );
        }
        Ok(source)
    }

    pub fn output(&self, snapshot: &Snapshot, args: &[&str]) -> Result<Vec<u8>> {
        let (command, run) = self.command(None, snapshot, args)?;
        let out = cmd::output(command, "chezmoi", args)?;
        run.finish(&self.user)?;
        Ok(out)
    }

    pub fn run(&self, recipients: &str, snapshot: &Snapshot, args: &[&str]) -> Result<()> {
        let (command, run) = self.command(Some(recipients), snapshot, args)?;
        cmd::status(command, "chezmoi", args)?;
        run.finish(&self.user)
    }

    fn command(
        &self,
        recipients: Option<&str>,
        snapshot: &Snapshot,
        args: &[&str],
    ) -> Result<(Cmd, Run)> {
        let dir = TempDir::new()?;
        let mut config = ChezmoiConfig {
            identity: dir.path().join("key.txt"),
            recipients: dir.path().join("recipients.txt"),
            ..self.config.clone()
        };
        if let Some(key) = secrets::user_key(&self.user)? {
            secrets::write_key(&config.identity, &key)?;
        }
        match recipients {
            Some(list) => fs::write(&config.recipients, list)?,
            None => config.recipients = self.config.recipients.clone(),
        }
        let config_path = dir.path().join("chezmoi.toml");
        fs::write(&config_path, config_text(&config)?)?;
        let state = dir.path().join("state.boltdb");
        if let Some(bytes) = self.user.read(&self.user.home.join(STATE))? {
            fs::write(&state, bytes)?;
        }
        let home = self.user.home.to_string_lossy().into_owned();
        let mut command = Cmd::from(Command::new("unshare"));
        command
            .args(["--mount", "--propagation", "private", "--", "sh", "-c"])
            .arg("mount --bind \"$1\" \"$2\" && shift 2 && exec \"$@\"")
            .arg("sh")
            .arg(snapshot.home())
            .args([&home, "chezmoi", "--config"])
            .arg(&config_path)
            .arg("--persistent-state")
            .arg(&state)
            .arg("--cache")
            .arg(dir.path().join("cache"))
            .args(["--destination", &home])
            .args(args)
            .env_clear()
            .envs([
                ("HOME", home.as_str()),
                ("PATH", user::SYSTEM_PATH),
                ("LANG", "C.UTF-8"),
                ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ])
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .env("XDG_CACHE_HOME", dir.path().join("cache"))
            .env("XDG_DATA_HOME", dir.path().join("data"))
            .env("XDG_STATE_HOME", dir.path().join("state"));
        Ok((command, Run { dir, state }))
    }
}

struct Run {
    dir: TempDir,
    state: PathBuf,
}

impl Run {
    fn finish(self, user: &User) -> Result<()> {
        if self.state.exists() {
            user.write(&user.home.join(STATE), &fs::read(&self.state)?)?;
        }
        self.dir
            .close()
            .context("removing chezmoi's temporary folder")
    }
}

pub fn init(fleet: &Fleet, member: &Member) -> Result<()> {
    let user = member.user();
    let text = config_text(&member.chezmoi_config(fleet))?;
    user.write(&user.home.join(CONFIG), text.as_bytes())
}

pub fn config_text(config: &ChezmoiConfig) -> Result<String> {
    Ok(toml::to_string(&serde_json::json!({
        "sourceDir": config.source,
        "encryption": "age",
        "age": {
            "useBuiltin": true,
            "identity": config.identity,
            "recipientsFile": config.recipients,
        },
        "data": { "profile": config.profile, "user": config.user, "fleet": config.fleet },
    }))?)
}

pub const CONFIG: &str = ".config/chezmoi/chezmoi.toml";
const STATE: &str = ".config/chezmoi/chezmoistate.boltdb";

pub fn download_deb(dir: &Path) -> Result<PathBuf> {
    let arch = cmd::stdout(As::Root, "dpkg", &["--print-architecture"])?;
    let suffix = format!("_linux_{}.deb", arch.trim());
    let latest = "https://api.github.com/repos/twpayne/chezmoi/releases/latest";
    let release: serde_json::Value =
        serde_json::from_str(&cmd::stdout(As::Root, "curl", &["-fsSL", latest])?)?;
    let asset = |wanted: &dyn Fn(&str) -> bool| -> Result<(String, String)> {
        release["assets"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|asset| asset["name"].as_str().is_some_and(wanted))
            .and_then(|asset| {
                let name = asset["name"].as_str()?.to_owned();
                Some((name, asset["browser_download_url"].as_str()?.to_owned()))
            })
            .context("chezmoi's latest release lacks the .deb or its checksums")
    };
    let (name, url) = asset(&|n| n.ends_with(&suffix))?;
    let (_, sums_url) = asset(&|n| n.ends_with("_checksums.txt"))?;
    let deb = dir.join(&name);
    cmd::root("curl", &["-fsSL", "-o", &deb.to_string_lossy(), &url])?;
    let sums = cmd::stdout(As::Root, "curl", &["-fsSL", &sums_url])?;
    let wanted = sums
        .lines()
        .find_map(|line| line.strip_suffix(name.as_str())?.split_whitespace().next())
        .with_context(|| format!("chezmoi's checksums don't list {name}"))?;
    let actual = cmd::stdout(As::Root, "sha256sum", &[&deb.to_string_lossy()])?;
    if actual.split_whitespace().next() != Some(wanted) {
        bail!("{name} doesn't match chezmoi's checksum");
    }
    Ok(deb)
}
