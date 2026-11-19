use crate::{
    cmd::{self, As},
    fleet::Fleet,
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

/// Writes into the fleet for the user, who can't write the checkout: root runs chezmoi in the
/// user's home with the user's config and state, so the user's chezmoi knows the file as one it
/// wrote. The state goes back to the user, and `Fleet::hand_back` gives the new files to the
/// admin.
pub fn write_for(user: &User, args: &[&str]) -> Result<()> {
    let cache = tempfile::TempDir::new()?;
    let config_dir = user.home.join(".config/chezmoi");
    let mut command = Command::new("chezmoi");
    command
        .env("HOME", &user.home)
        .arg("--config")
        .arg(config_dir.join("chezmoi.toml"))
        .arg("--cache")
        .arg(cache.path())
        .args(args);
    let result = cmd::status(command, "chezmoi", args);
    user.own(&config_dir.join("chezmoistate.boltdb"))?;
    result
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Managed {
    source_absolute: PathBuf,
}

pub fn encrypted(fleet: &Fleet, type_name: &str, user: &str) -> Result<Vec<(PathBuf, PathBuf)>> {
    let dir = tempfile::TempDir::new()?;
    // Without encryption set, chezmoi keeps .age in the names.
    let config = dir.path().join("chezmoi.toml");
    fs::write(&config, "encryption = \"age\"\n[age]\nuseBuiltin = true\n")?;
    let data = serde_json::json!({ "profile": type_name, "user": user }).to_string();
    let (config, state, home) = (
        config.to_string_lossy().into_owned(),
        dir.path().join("state").to_string_lossy().into_owned(),
        dir.path().join("home").to_string_lossy().into_owned(),
    );
    let source = fleet.dir.to_string_lossy();
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

/// Debian has no package for chezmoi: the .deb comes from its GitHub releases, checked against
/// the release's checksums.
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
