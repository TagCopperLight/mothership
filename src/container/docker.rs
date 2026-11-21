use super::{Image, Pin, account, registry};
use crate::{
    cmd::{self, As},
    user::User,
};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

const QUERY_LIMIT: Duration = Duration::from_secs(120);
const RUN_LIMIT: Duration = Duration::from_secs(60 * 60);
const HEALTH_LIMIT: Duration = Duration::from_secs(10 * 60);
const HEALTH_POLL: Duration = Duration::from_secs(2);
const PROJECT_LABEL: &str = "com.docker.compose.project";

pub struct Project<'a> {
    pub name: &'a str,
    pub dir: PathBuf,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, PartialEq)]
pub enum Health {
    Healthy,
    Unchecked,
    Unhealthy(String),
}

pub trait Engine {
    fn config(&self, project: &Project) -> Result<Value>;
    fn pull(&self, project: &Project) -> Result<()>;
    fn up(&self, project: &Project) -> Result<()>;
    fn stop(&self, name: &str) -> Result<()>;
    fn down(&self, name: &str) -> Result<()>;
    fn health(&self, name: &str) -> Result<Health>;
    fn runs(&self, name: &str) -> Result<bool>;
    fn snapshot(&self, name: &str, data: &Path) -> Result<()>;
    fn restore(&self, name: &str, data: &Path) -> Result<()>;
    fn confirm(&self, image: &Image, pin: &Pin) -> Result<Option<String>>;
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: ContainerState,
    pub config: ContainerConfig,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContainerState {
    pub status: String,
    pub exit_code: i64,
    pub health: Option<ContainerHealth>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContainerHealth {
    pub status: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContainerConfig {
    #[serde(default)]
    pub labels: Option<std::collections::BTreeMap<String, String>>,
}

impl Container {
    pub fn project(&self) -> Option<&str> {
        self.config
            .labels
            .as_ref()?
            .get(PROJECT_LABEL)
            .map(String::as_str)
    }

    pub fn shown(&self) -> &str {
        self.name.trim_start_matches('/')
    }
}

pub struct Docker {
    pub user: User,
    pub registry: registry::Client,
}

impl Docker {
    pub fn find(registry: registry::Client) -> Result<Option<Docker>> {
        Ok(account::user()?.map(|user| Docker { user, registry }))
    }

    fn command<S: AsRef<std::ffi::OsStr>>(&self, args: &[S]) -> Command {
        let mut command = As::User(&self.user).command("docker", args);
        command.env("DOCKER_HOST", account::socket(&self.user));
        command
    }

    fn query<S: AsRef<std::ffi::OsStr>>(&self, args: &[S]) -> Result<Vec<u8>> {
        cmd::output_within(self.command(args), "docker", args, QUERY_LIMIT)
    }

    fn compose(&self, project: &Project, verb: &[&str]) -> (Command, Vec<OsString>) {
        let mut args: Vec<OsString> = ["compose", "--project-name", project.name]
            .map(OsString::from)
            .into();
        args.push("--project-directory".into());
        args.push(project.dir.clone().into());
        for file in [super::COMPOSE, super::deploy::OVERRIDE] {
            args.push("-f".into());
            args.push(project.dir.join(file).into());
        }
        args.extend(verb.iter().map(OsString::from));
        let mut command = self.command(&args);
        command.envs(project.env.iter().map(|(k, v)| (k, v)));
        (command, args)
    }

    fn run_compose(&self, project: &Project, verb: &[&str]) -> Result<()> {
        let (command, args) = self.compose(project, verb);
        cmd::status_within(command, "docker", &args, RUN_LIMIT)
    }

    pub fn answers(&self) -> bool {
        self.query(&["info", "--format", "{{.ServerVersion}}"])
            .is_ok()
    }

    pub fn containers(&self) -> Result<Vec<Container>> {
        let ids = String::from_utf8(self.query(&["ps", "--all", "--quiet", "--no-trunc"])?)?;
        let ids: Vec<&str> = ids.split_whitespace().collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let args = [&["container", "inspect"][..], &ids].concat();
        serde_json::from_slice(&self.query(&args)?).context("reading docker inspect")
    }

    pub fn image_id(&self, reference: &str) -> Result<Option<String>> {
        let args = ["image", "inspect", "--format", "{{.Id}}", reference];
        match self.query(&args) {
            Ok(out) => Ok(Some(String::from_utf8(out)?.trim().to_owned())),
            Err(_) => Ok(None),
        }
    }

    pub fn remove_container(&self, name: &str) -> Result<()> {
        self.query(&["rm", "--force", "--volumes", "--", name])
            .map(drop)
    }

    fn of(&self, name: &str) -> Result<Vec<Container>> {
        Ok(self
            .containers()?
            .into_iter()
            .filter(|c| c.project() == Some(name))
            .collect())
    }

    fn in_namespace(&self, script: &str, args: &[&Path]) -> Result<()> {
        let pid_file = account::runtime(&self.user).join("docker.pid");
        let pid = self.user.read(&pid_file)?.with_context(|| {
            format!(
                "{} is missing: is rootless Docker running?",
                pid_file.display()
            )
        })?;
        let pid = String::from_utf8(pid)?.trim().to_owned();
        if pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) {
            bail!("{} doesn't hold a process id", pid_file.display());
        }
        let mut all: Vec<OsString> = [
            "-U",
            "--preserve-credentials",
            "-m",
            "-t",
            &pid,
            "--",
            "sh",
            "-euc",
            script,
            "sh",
        ]
        .map(OsString::from)
        .into();
        all.extend(args.iter().map(|p| p.as_os_str().to_owned()));
        let command = As::User(&self.user).command("nsenter", &all);
        cmd::status_within(command, "nsenter", &all, RUN_LIMIT)
    }
}

impl Engine for Docker {
    fn config(&self, project: &Project) -> Result<Value> {
        let (command, args) = self.compose(project, &["config", "--format", "json"]);
        let out = cmd::output_within(command, "docker", &args, QUERY_LIMIT)?;
        serde_json::from_slice(&out).context("reading docker compose config")
    }

    fn pull(&self, project: &Project) -> Result<()> {
        self.run_compose(project, &["pull", "--quiet"])
    }

    fn up(&self, project: &Project) -> Result<()> {
        self.run_compose(
            project,
            &["up", "--detach", "--remove-orphans", "--pull", "never"],
        )
    }

    fn stop(&self, name: &str) -> Result<()> {
        let args = ["compose", "--project-name", name, "stop"];
        cmd::status_within(self.command(&args), "docker", &args, RUN_LIMIT)
    }

    fn down(&self, name: &str) -> Result<()> {
        let args = [
            "compose",
            "--project-name",
            name,
            "down",
            "--remove-orphans",
        ];
        cmd::status_within(self.command(&args), "docker", &args, RUN_LIMIT)
    }

    fn runs(&self, name: &str) -> Result<bool> {
        Ok(self.of(name)?.iter().any(|c| c.state.status == "running"))
    }

    fn health(&self, name: &str) -> Result<Health> {
        let start = Instant::now();
        loop {
            let containers = self.of(name)?;
            if containers.iter().all(|c| c.state.health.is_none()) {
                return Ok(Health::Unchecked);
            }
            if let Some(why) = sick(&containers) {
                return Ok(Health::Unhealthy(why));
            }
            let healthy = containers.iter().all(|c| {
                c.state
                    .health
                    .as_ref()
                    .is_none_or(|h| h.status == "healthy")
            });
            if healthy {
                return Ok(Health::Healthy);
            }
            if start.elapsed() > HEALTH_LIMIT {
                return Ok(Health::Unhealthy(format!(
                    "not healthy after {} minutes",
                    HEALTH_LIMIT.as_secs() / 60
                )));
            }
            thread::sleep(HEALTH_POLL);
        }
    }

    fn snapshot(&self, name: &str, data: &Path) -> Result<()> {
        let to = account::snapshot(&self.user, name);
        self.in_namespace(
            "rm -rf \"$2.new\"; mkdir -p \"$2.new\"; cp -a \"$1/.\" \"$2.new/\"; rm -rf \"$2\"; mv \"$2.new\" \"$2\"",
            &[data, &to],
        )
    }

    fn restore(&self, name: &str, data: &Path) -> Result<()> {
        let from = account::snapshot(&self.user, name);
        self.in_namespace(
            "find \"$1\" -mindepth 1 -delete; cp -a \"$2/.\" \"$1/\"",
            &[data, &from],
        )
    }

    fn confirm(&self, image: &Image, pin: &Pin) -> Result<Option<String>> {
        let digest = self.registry.digest(&image.image, &pin.version)?;
        Ok((digest != pin.digest).then(|| {
            format!(
                "the registry gives {digest} for {}:{}, not the {} the fleet pins",
                image.image, pin.version, pin.digest
            )
        }))
    }
}

fn sick(containers: &[Container]) -> Option<String> {
    containers.iter().find_map(|c| {
        let health = c.state.health.as_ref().map(|h| h.status.as_str());
        match (c.state.status.as_str(), health) {
            (_, Some("unhealthy")) => Some(format!("{} reports unhealthy", c.shown())),
            ("restarting", _) => Some(format!("{} keeps restarting", c.shown())),
            ("exited" | "dead", _) if c.state.exit_code != 0 => Some(format!(
                "{} stopped with code {}",
                c.shown(),
                c.state.exit_code
            )),
            _ => None,
        }
    })
}
