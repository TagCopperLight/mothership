use super::{Image, Pin, account, link, registry};
use crate::{
    system::user::User,
    util::cmd::{self, As},
};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
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
const SETTLE: Duration = Duration::from_secs(30);
const PROJECT_LABEL: &str = "com.docker.compose.project";

pub struct Project<'a> {
    pub name: &'a str,
    pub dir: PathBuf,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, PartialEq)]
pub enum Health {
    Healthy,
    Unhealthy(String),
}

pub trait Engine {
    fn config(&self, project: &Project) -> Result<Value>;
    fn pull(&self, project: &Project) -> Result<()>;
    fn up(&self, project: &Project) -> Result<()>;
    fn stop(&self, name: &str) -> Result<()>;
    fn start(&self, name: &str) -> Result<()>;
    fn down(&self, name: &str) -> Result<()>;
    fn health(&self, name: &str) -> Result<Health>;
    fn snapshot(&self, name: &str, data: &Path, parts: &[PathBuf]) -> Result<()>;
    fn restore(&self, name: &str, data: &Path, parts: &[PathBuf]) -> Result<()>;
    fn drop_snapshot(&self, name: &str) -> Result<()>;
    fn confirm(&self, image: &Image, pin: &Pin) -> Result<Option<String>>;
    fn link(&self, network: &str) -> Result<()>;
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: ContainerState,
    pub restart_count: i64,
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
    pub labels: Option<BTreeMap<String, String>>,
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

    pub fn links(&self) -> Result<Vec<String>> {
        let out = self.query(&["network", "ls", "--format", "{{.Name}}"])?;
        Ok(String::from_utf8(out)?
            .lines()
            .filter(|name| name.starts_with(link::PREFIX))
            .map(str::to_owned)
            .collect())
    }

    fn attached(&self, network: &str) -> Result<Vec<String>> {
        let filter = format!("network={network}");
        let out = self.query(&["ps", "--all", "--quiet", "--no-trunc", "--filter", &filter])?;
        Ok(String::from_utf8(out)?
            .split_whitespace()
            .map(str::to_owned)
            .collect())
    }

    pub fn remove_link(&self, network: &str) -> Result<()> {
        for container in self.attached(network)? {
            self.query(&["network", "disconnect", "--force", network, &container])?;
        }
        self.query(&["network", "rm", "--", network]).map(drop)
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

    fn start(&self, name: &str) -> Result<()> {
        let args = ["compose", "--project-name", name, "start"];
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

    fn health(&self, name: &str) -> Result<Health> {
        let start = Instant::now();
        let mut restarts_at_start = BTreeMap::new();
        loop {
            let containers = self.of(name)?;
            for c in &containers {
                restarts_at_start
                    .entry(c.id.clone())
                    .or_insert(c.restart_count);
            }
            if let Some(why) = sick(&containers, &restarts_at_start) {
                return Ok(Health::Unhealthy(why));
            }
            let healthy = containers.iter().all(|c| {
                c.state
                    .health
                    .as_ref()
                    .is_none_or(|h| h.status == "healthy")
            });
            if healthy && start.elapsed() >= SETTLE {
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

    fn snapshot(&self, name: &str, data: &Path, parts: &[PathBuf]) -> Result<()> {
        let to = account::snapshot(&self.user, name);
        self.in_namespace(&format!("{PARTS}{SNAPSHOT}"), &with_parts(data, &to, parts))
    }

    fn restore(&self, name: &str, data: &Path, parts: &[PathBuf]) -> Result<()> {
        let from = account::snapshot(&self.user, name);
        self.in_namespace(
            &format!("{PARTS}{RESTORE}"),
            &with_parts(data, &from, parts),
        )
    }

    fn drop_snapshot(&self, name: &str) -> Result<()> {
        let snapshot = account::snapshot(&self.user, name);
        self.in_namespace(DROP_SNAPSHOT, &[&snapshot])
    }

    fn confirm(&self, image: &Image, pin: &Pin) -> Result<Option<String>> {
        self.registry.contradicts(image, pin)
    }

    fn link(&self, network: &str) -> Result<()> {
        if self.query(&["network", "inspect", "--", network]).is_ok() {
            return Ok(());
        }
        self.query(&["network", "create", "--internal", "--", network])
            .map(drop)
    }
}

const PARTS: &str = r#"d=$1 s=$2; shift 2; [ $# -gt 0 ] || set -- .
inside() {
  b=$(realpath -m -- "$1")
  case "$(realpath -m -- "$1/$2")" in "$b" | "$b"/*) ;; *) echo "$1/$2 leads out of $1" >&2; exit 1 ;; esac
}
"#;

const SNAPSHOT: &str = r#"rm -rf "$s.new"; mkdir -p "$s.new"
for p; do
  inside "$d" "$p"
  if [ -e "$d/$p" ]; then mkdir -p "$s.new/$(dirname "$p")"; cp -a "$d/$p" "$s.new/$(dirname "$p")/"; fi
done
rm -rf "$s"; mv "$s.new" "$s""#;

const DROP_SNAPSHOT: &str = r#"rm -rf -- "$1" "$1.new""#;

const RESTORE: &str = r#"for p; do
  inside "$d" "$p"
  if [ "$p" = . ]; then find "$d" -mindepth 1 -delete; else rm -rf "$d/$p"; fi
  if [ -e "$s/$p" ]; then mkdir -p "$d/$(dirname "$p")"; cp -a "$s/$p" "$d/$(dirname "$p")/"; fi
done"#;

fn with_parts<'a>(data: &'a Path, snapshot: &'a Path, parts: &'a [PathBuf]) -> Vec<&'a Path> {
    [data, snapshot]
        .into_iter()
        .chain(parts.iter().map(PathBuf::as_path))
        .collect()
}

fn sick(containers: &[Container], restarts_at_start: &BTreeMap<String, i64>) -> Option<String> {
    containers.iter().find_map(|c| {
        let health = c.state.health.as_ref().map(|h| h.status.as_str());
        let restarted = restarts_at_start
            .get(&c.id)
            .is_some_and(|&n| c.restart_count > n);
        match (c.state.status.as_str(), health) {
            (_, Some("unhealthy")) => Some(format!("{} reports unhealthy", c.shown())),
            ("restarting", _) => Some(format!("{} keeps restarting", c.shown())),
            _ if restarted => Some(format!("{} crashed and restarted", c.shown())),
            ("exited" | "dead", _) if c.state.exit_code != 0 => Some(format!(
                "{} stopped with code {}",
                c.shown(),
                c.state.exit_code
            )),
            _ => None,
        }
    })
}

#[cfg(test)]
#[path = "../../tests/services/docker.rs"]
mod tests;
