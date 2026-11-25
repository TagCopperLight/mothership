mod plan;
mod rebuild;
pub mod state;
pub mod unit;

use crate::{
    fleet::{
        git::{GIT_DIR, Git, short},
        sync::CHECKED,
    },
    services::{self, recipe, registry::Client},
    trust::github,
    trust::sign,
    trust::verify,
    util::file,
    util::http::{Curl, Http, Request},
};
use anyhow::{Context, Result, bail};
use plan::{Update, Watched};
use services::Pin;
use state::State;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const TIP: &str = "refs/probe/main";

struct Setting {
    credentials: PathBuf,
    state: PathBuf,
    fleet: String,
    host: String,
}

fn setting() -> Result<Setting> {
    let var = |name: &str| {
        std::env::var(name)
            .with_context(|| format!("{name} is unset: the probe runs from its systemd unit"))
    };
    Ok(Setting {
        credentials: PathBuf::from(var("CREDENTIALS_DIRECTORY")?),
        state: PathBuf::from(var("STATE_DIRECTORY")?),
        fleet: var(unit::FLEET_VAR)?,
        host: var(unit::HOST_VAR)?,
    })
}

pub fn run() -> Result<()> {
    let setting = setting()?;
    let path = setting.state.join(state::FILE);
    let mut state = State::load(&path)?;
    let result = round(&setting, &mut state);
    state.problem = result.as_ref().err().map(|e| format!("{e:#}"));
    state.save(&path)?;
    result
}

fn round(setting: &Setting, state: &mut State) -> Result<()> {
    let dir = setting.state.join("fleet.git");
    let ssh = format!(
        "ssh -i {} -o IdentitiesOnly=yes -o BatchMode=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile={}",
        setting.credentials.join(unit::SSH).display(),
        known_hosts(setting)?.display()
    );
    let git = Git::at(&dir).with_env("GIT_SSH_COMMAND", ssh);
    if !dir.exists() {
        git.out(&["init", "--quiet", "--bare"])?;
        git.out(&["config", "fetch.fsckObjects", "true"])?;
    }
    git.out(&[
        "fetch",
        "--quiet",
        "--no-tags",
        &setting.fleet,
        &format!("+refs/heads/main:{TIP}"),
    ])?;
    let tip = git.rev(TIP)?.context("the fleet has no main")?;
    let anchor = root_checked()?;
    let checked = verify::check(&git, &anchor, &tip)?;
    if let Some(refused) = checked.refused {
        bail!(
            "machines refuse {} \"{}\" on the fleet's main, so the probe waits: {}",
            short(&refused.commit),
            refused.subject,
            refused.why
        );
    }
    let watched = watched(&git, &tip)?;
    let credential = |name: &str| file::read(&setting.credentials.join(name));
    let registry = Client::new(Box::new(Curl), credential(unit::LOGINS)?.as_deref())?;
    let now = now()?;
    let round = plan::Round::at(state, now);
    let verify = |item: &Watched, pin: &Pin| match recipe::built_from(
        &git,
        &tip,
        &registry,
        &item.service,
        &item.image,
        pin,
    )? {
        Some(why) => bail!("{why}"),
        None => Ok(()),
    };
    let found = plan::plan(&watched, &registry, state, now, round, &verify);
    for notice in state.notices.values() {
        println!("{notice}");
    }
    let mut failures = found.failures;
    if round == plan::Round::Everything
        && let Some(token) = credential(unit::GITHUB_TOKEN)?
    {
        let github = rebuild::GitHub {
            http: &Curl,
            token: String::from_utf8(token)?.trim().to_owned(),
        };
        let fleet_repo = github::repo(&setting.fleet);
        let done = rebuild::ask(&github, &registry, &watched, fleet_repo.as_deref(), state);
        for asked in done.asked {
            println!("Asked to rebuild {asked}, since a base image changed");
        }
        failures.extend(done.failures);
    }
    let failed = || match failures.is_empty() {
        true => Ok(()),
        false => Err(anyhow::anyhow!(
            "the probe couldn't follow {}",
            failures.join("\n")
        )),
    };
    if found.updates.is_empty() {
        println!("Every image is up to date.");
        return failed();
    }
    let signing = setting.credentials.join(unit::SIGNING);
    let bytes = credential(unit::SIGNING)?.context("the probe has no signing key")?;
    let key = sign::parse_key(&bytes, &signing)?;
    let commit = commit(
        &git,
        &dir,
        &tip,
        &anchor,
        &found.updates,
        &setting.host,
        &key,
    )?;
    git.out(&[
        "push",
        "--quiet",
        &setting.fleet,
        &format!("{commit}:refs/heads/main"),
    ])?;
    println!("Pushed {}", short(&commit));
    failed()
}

fn known_hosts(setting: &Setting) -> Result<PathBuf> {
    if github::repo(&setting.fleet).is_none() {
        return Ok(PathBuf::from("/dev/null"));
    }
    let response = Curl
        .send(&Request {
            method: "GET",
            url: github::META,
            headers: vec!["User-Agent: mothership-probe".to_owned()],
            body: None,
        })?
        .ok("GitHub's metadata")?;
    let lines = github::known_hosts(&response.body)?;
    let path = setting.state.join("known_hosts");
    file::write(&path, format!("{}\n", lines.join("\n")).as_bytes(), 0o644)?;
    Ok(path)
}

fn root_checked() -> Result<String> {
    let git = Path::new(GIT_DIR);
    if let Some(text) = file::read_if_any(&git.join(CHECKED))? {
        return Ok(text.trim().to_owned());
    }
    let packed = file::read_if_any(&git.join("packed-refs"))?.unwrap_or_default();
    packed
        .lines()
        .find_map(|line| line.strip_suffix(&format!(" {CHECKED}")))
        .map(str::to_owned)
        .context("this machine has checked no commit of the fleet yet")
}

fn now() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs())
}

fn watched(git: &Git, commit: &str) -> Result<Vec<Watched>> {
    let mut watched = Vec::new();
    for path in git.files(commit, crate::fleet::TYPES)? {
        let Some(service) = services::service_of(&path) else {
            continue;
        };
        let path = path.to_str().expect("a service file's path is UTF-8");
        let text = git.blob(commit, path)?.context("ls-tree listed it")?;
        let images = services::parse(&text).with_context(|| format!("parsing {path}"))?;
        for (name, image) in images {
            let dockerfile = match image.recipe {
                true => git.blob(commit, &recipe::path(&service))?,
                false => None,
            };
            watched.push(Watched {
                service: service.clone(),
                name,
                image,
                dockerfile,
            });
        }
    }
    Ok(watched)
}

fn commit(
    git: &Git,
    dir: &Path,
    base: &str,
    accepted: &str,
    updates: &[Update],
    host: &str,
    key: &ssh_key::PrivateKey,
) -> Result<String> {
    let mut texts: BTreeMap<String, toml_edit::DocumentMut> = BTreeMap::new();
    for update in updates {
        let (type_name, name) = update
            .service
            .split_once('/')
            .expect("service_of names both");
        let path = format!(
            "types/{type_name}/{}/{name}/{}",
            services::SERVICES,
            services::SERVICE_TOML
        );
        if !texts.contains_key(&path) {
            let text = git.blob(base, &path)?.context("the probe read it")?;
            texts.insert(path.clone(), text.parse()?);
        }
        let image = &mut texts.get_mut(&path).expect("just read")["images"][&update.name];
        image["version"] = toml_edit::value(update.to.version.as_str());
        image["digest"] = toml_edit::value(update.to.digest.as_str());
    }
    let index = tempfile::TempDir::new()?;
    let indexed = Git::at(dir).with_env(
        "GIT_INDEX_FILE",
        index.path().join("index").display().to_string(),
    );
    indexed.out(&["read-tree", base])?;
    for (path, text) in &texts {
        let blob = indexed.with_input(&["hash-object", "-w", "--stdin"], &text.to_string())?;
        indexed.out(&[
            "update-index",
            "--cacheinfo",
            &format!("100644,{blob},{path}"),
        ])?;
    }
    let tree = indexed.out(&["write-tree"])?;
    let lines: Vec<String> = updates
        .iter()
        .map(|u| match u.from.version == u.to.version {
            true => format!(
                "{} {}: {} with a new digest",
                u.service, u.name, u.to.version
            ),
            false => format!(
                "{} {}: {} to {}",
                u.service, u.name, u.from.version, u.to.version
            ),
        })
        .collect();
    let message = format!(
        "Update {}\n\n{}",
        crate::util::style::count(lines.len(), "image"),
        lines.join("\n")
    );
    let who = format!("{host}/probe <probe@{host}>");
    verify::signed_commit(git, &tree, base, accepted, &who, &message, key)
}

#[cfg(test)]
#[path = "../../tests/probe/mod.rs"]
mod tests;
