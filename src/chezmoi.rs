use crate::{
    cmd::{self, As},
    fleet::{ChezmoiConfig, Fleet, Member},
    homes,
    user::User,
};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub fn managed(user: &User, include: &str) -> Result<Vec<PathBuf>> {
    let args = ["managed", "--include", include, "--path-style", "absolute"];
    Ok(cmd::stdout(As::User(user), "chezmoi", &args)?
        .lines()
        .map(PathBuf::from)
        .collect())
}

pub fn source_path(user: &User, target: &std::path::Path) -> Result<PathBuf> {
    let target = target.to_string_lossy();
    let out = cmd::stdout(As::User(user), "chezmoi", &["source-path", &target])?;
    Ok(PathBuf::from(out.trim()))
}

pub fn write_for(fleet: &Fleet, member: &Member, recipients: &str, args: &[&str]) -> Result<()> {
    let user = member.user();
    let dir = tempfile::TempDir::new()?;
    let list = dir.path().join("recipients.txt");
    fs::write(&list, recipients)?;
    let config = dir.path().join("chezmoi.toml");
    let wanted = ChezmoiConfig {
        recipients: list,
        ..member.chezmoi_config(fleet)
    };
    fs::write(&config, config_text(&wanted)?)?;
    let home_state = user.home.join(STATE);
    let state = dir.path().join("state.boltdb");
    if let Some(bytes) = user.read(&home_state)? {
        fs::write(&state, bytes)?;
    }
    let mut command = Command::new("chezmoi");
    command
        .env("HOME", &user.home)
        .arg("--config")
        .arg(&config)
        .arg("--persistent-state")
        .arg(&state)
        .arg("--cache")
        .arg(dir.path().join("cache"))
        .arg("--destination")
        .arg(&user.home)
        .args(args);
    cmd::status(command, "chezmoi", args)?;
    match state.exists() {
        true => user.write(&home_state, &fs::read(&state)?),
        false => Ok(()),
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
        "data": data(&config.profile, &config.user, &config.fleet),
    }))?)
}

pub const CONFIG: &str = ".config/chezmoi/chezmoi.toml";
const STATE: &str = ".config/chezmoi/chezmoistate.boltdb";

fn data(profile: &str, user: &str, fleet: &Path) -> serde_json::Value {
    serde_json::json!({ "profile": profile, "user": user, "fleet": fleet })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Managed {
    source_absolute: PathBuf,
}

pub fn encrypted(fleet: &Fleet, type_name: &str, user: &str) -> Result<Vec<(PathBuf, PathBuf)>> {
    let dir = tempfile::TempDir::new()?;
    let config = dir.path().join("chezmoi.toml");
    fs::write(&config, "encryption = \"age\"\n[age]\nuseBuiltin = true\n")?;
    let data = data(type_name, user, &fleet.dir).to_string();
    let (config, state, home) = (
        config.to_string_lossy().into_owned(),
        dir.path().join("state").to_string_lossy().into_owned(),
        dir.path().join("home").to_string_lossy().into_owned(),
    );
    let source = homes::source(&fleet.dir, user);
    let source = source.to_string_lossy();
    let args = [
        "--config",
        &config,
        "--persistent-state",
        &state,
        "--source",
        &source,
        "--destination",
        &home,
        "--override-data",
        &data,
        "managed",
        "--include",
        "encrypted",
        "--path-style",
        "all",
        "--format",
        "json",
    ];
    let found: BTreeMap<PathBuf, Managed> =
        serde_json::from_str(&cmd::stdout(As::Root, "chezmoi", &args)?)?;
    Ok(found
        .into_iter()
        .map(|(target, m)| (target, m.source_absolute))
        .collect())
}

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
