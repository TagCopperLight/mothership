use crate::{
    cmd::{self, As},
    fleet::Fleet,
    user::User,
};
use anyhow::Result;
use serde::Deserialize;
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command};

/// What the user's chezmoi manages, by their absolute path. `include` is chezmoi's entry types.
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

/// The encrypted files chezmoi gives `user` on `type_name`, with their paths in the home, read
/// from the fleet's rules rather than a machine's.
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
