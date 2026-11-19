use crate::{
    cmd::{self, As, Cmd},
    secrets,
    snapshot::Snapshot,
    user::{self, User},
};
use anyhow::{Context, Result, bail};
use std::{
    ffi::OsStr,
    fs,
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;

pub fn managed(user: &User, include: &str) -> Result<Vec<PathBuf>> {
    let args = ["managed", "--include", include, "--path-style", "absolute"];
    lines(&cmd::bytes(As::User(user), "chezmoi", &args)?)
        .map(|line| in_home(user, line))
        .collect()
}

pub fn manages(user: &User, target: &Path) -> Result<bool> {
    let args = [
        OsStr::new("managed"),
        OsStr::new("--path-style"),
        OsStr::new("absolute"),
        target.as_os_str(),
    ];
    let out = cmd::bytes(As::User(user), "chezmoi", &args)?;
    Ok(lines(&out).any(|line| line == target))
}

fn lines(out: &[u8]) -> impl Iterator<Item = &Path> {
    out.split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| Path::new(OsStr::from_bytes(line)))
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
pub struct ChezmoiConfig {
    pub source: PathBuf,
    pub identity: PathBuf,
    pub recipients: PathBuf,
    pub profile: String,
    pub user: String,
    pub fleet: PathBuf,
}

#[derive(Clone)]
pub struct Root {
    user: User,
    config: ChezmoiConfig,
}

impl Root {
    pub fn new(user: &User, config: ChezmoiConfig) -> Root {
        Root {
            user: user.clone(),
            config,
        }
    }

    pub fn user(&self) -> &User {
        &self.user
    }

    pub fn source_path(&self, target: &Path) -> Result<PathBuf> {
        let empty = Snapshot::take(&self.user, &[])?;
        let out = self.output(&empty, &[OsStr::new("source-path"), target.as_os_str()])?;
        let source = Path::new(OsStr::from_bytes(out.trim_ascii_end())).to_path_buf();
        let inside = source
            .strip_prefix(&self.config.source)
            .is_ok_and(|rel| rel.components().all(|c| matches!(c, Component::Normal(_))));
        if !inside {
            bail!(
                "chezmoi put {} outside {}",
                target.display(),
                self.config.source.display()
            );
        }
        Ok(source)
    }

    pub fn output<S: AsRef<OsStr>>(&self, snapshot: &Snapshot, args: &[S]) -> Result<Vec<u8>> {
        let (command, run) = self.command(None, snapshot, args)?;
        let out = cmd::output(command, "chezmoi", args)?;
        run.finish()?;
        Ok(out)
    }

    pub fn run<S: AsRef<OsStr>>(
        &self,
        recipients: &str,
        snapshot: &Snapshot,
        args: &[S],
    ) -> Result<()> {
        let (command, run) = self.command(Some(recipients), snapshot, args)?;
        cmd::status(command, "chezmoi", args)?;
        run.finish()
    }

    fn command<S: AsRef<OsStr>>(
        &self,
        recipients: Option<&str>,
        snapshot: &Snapshot,
        args: &[S],
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
            .arg(dir.path().join("state.boltdb"))
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
        Ok((command, Run { dir }))
    }
}

struct Run {
    dir: TempDir,
}

impl Run {
    fn finish(self) -> Result<()> {
        self.dir
            .close()
            .context("removing chezmoi's temporary folder")
    }
}

pub fn init(user: &User, config: &ChezmoiConfig) -> Result<()> {
    user.write(&user.home.join(CONFIG), config_text(config)?.as_bytes())
}

pub fn configured(user: &User, config: &ChezmoiConfig) -> Result<bool> {
    let Some(text) = user.read(&user.home.join(CONFIG))? else {
        return Ok(false);
    };
    Ok(String::from_utf8_lossy(&text) == config_text(config)?)
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

#[cfg(test)]
#[path = "../tests/chezmoi.rs"]
mod tests;
