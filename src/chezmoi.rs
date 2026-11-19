use crate::{
    cmd::{self, As},
    file, secrets,
    snapshot::Snapshot,
    user::{self, User},
};
use anyhow::{Context, Result, bail};
use std::{
    ffi::OsStr,
    fs,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
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
    if file::plain_within(&user.home, path).is_none() {
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
        if file::plain_within(&self.config.source, &source).is_none() {
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
        run.finish(&self.user)?;
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
        run.finish(&self.user)
    }

    fn command<S: AsRef<OsStr>>(
        &self,
        recipients: Option<&str>,
        snapshot: &Snapshot,
        args: &[S],
    ) -> Result<(Command, Run)> {
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
        let mut command = Command::new("unshare");
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

pub fn init(user: &User, config: &ChezmoiConfig) -> Result<()> {
    user.write(&user.home.join(CONFIG), config_text(config)?.as_bytes())
}

pub fn configured(user: &User, config: &ChezmoiConfig) -> Result<bool> {
    let Some(text) = user.read(&user.home.join(CONFIG))? else {
        return Ok(false);
    };
    Ok(String::from_utf8_lossy(&text) == config_text(config)?)
}

fn config_text(config: &ChezmoiConfig) -> Result<String> {
    Ok(toml::to_string(&serde_json::json!({
        "sourceDir": config.source,
        "encryption": "age",
        "age": {
            "useBuiltin": true,
            "identity": config.identity,
            "recipientsFile": config.recipients,
        },
        "data": { "profile": config.profile, "user": config.user },
    }))?)
}

const ATTRIBUTES: [&str; 11] = [
    "create_",
    "modify_",
    "remove_",
    ENCRYPTED,
    "private_",
    "readonly_",
    "empty_",
    "executable_",
    "exact_",
    "external_",
    "symlink_",
];

pub fn target_of(source: &Path) -> (PathBuf, bool) {
    let mut parts: Vec<String> = Vec::new();
    let mut encrypted = false;
    for part in source.iter() {
        let mut name = part.to_string_lossy().into_owned();
        while let Some(rest) = ATTRIBUTES.iter().find_map(|a| name.strip_prefix(a)) {
            encrypted |= name.starts_with(ENCRYPTED);
            name = rest.to_owned();
        }
        parts.push(match name.strip_prefix("dot_") {
            Some(rest) => format!(".{rest}"),
            None => name,
        });
    }
    if let Some(last) = parts.last_mut() {
        while let Some(rest) = last
            .strip_suffix(secrets::AGE)
            .or_else(|| last.strip_suffix(TEMPLATE))
        {
            *last = rest.to_owned();
        }
    }
    (parts.iter().collect(), encrypted)
}

const CONFIG: &str = ".config/chezmoi/chezmoi.toml";
const STATE: &str = ".config/chezmoi/chezmoistate.boltdb";
pub const ENCRYPTED: &str = "encrypted_";
pub const TEMPLATE: &str = ".tmpl";

#[cfg(test)]
#[path = "../tests/chezmoi.rs"]
mod tests;
