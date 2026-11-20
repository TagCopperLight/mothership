use crate::{
    cmd::{self, As},
    file, fleet, hash,
};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{env, fs, os::unix::process::CommandExt, path::Path, process::Command};

pub const BINARY: &str = "/usr/local/bin/mothership";
const PIN: &str = "mothership.toml";
const RELEASES: &str = "https://github.com/TagCopperLight/mothership/releases/download";
const ASSET: &str = "mothership-x86_64";

#[derive(Deserialize)]
struct Pin {
    version: String,
    sha256: String,
}

pub enum Outcome {
    Current,
    Declined,
}

pub fn follow_fleet(approve: &mut dyn FnMut() -> Result<bool>) -> Result<Outcome> {
    let Some(pin) = pin(Path::new(fleet::DIR))? else {
        return Ok(Outcome::Current);
    };
    let installed =
        hash::sha256_file(Path::new(BINARY)).with_context(|| format!("reading {BINARY}"))?;
    if installed.as_deref() == Some(pin.sha256.as_str()) {
        return Ok(Outcome::Current);
    }
    println!("The fleet runs mothership {}.", pin.version);
    if !approve()? {
        return Ok(Outcome::Declined);
    }
    let restart = running_installed()?;
    file::write(Path::new(BINARY), &download(&pin)?, 0o755)?;
    if restart {
        let error = Command::new(BINARY).args(env::args_os().skip(1)).exec();
        return Err(error).with_context(|| format!("starting {BINARY}"));
    }
    Ok(Outcome::Current)
}

fn pin(fleet: &Path) -> Result<Option<Pin>> {
    let path = fleet.join(PIN);
    file::read_if_any(&path)?
        .map(|text| toml::from_str(&text).with_context(|| format!("reading {}", path.display())))
        .transpose()
}

fn download(pin: &Pin) -> Result<Vec<u8>> {
    if env::consts::ARCH != "x86_64" {
        bail!("mothership's releases hold only an x86_64 binary");
    }
    let url = format!("{RELEASES}/v{}/{ASSET}", pin.version);
    let bytes = cmd::bytes(As::Root, "curl", &["-fsSL", &url])?;
    if hash::sha256(&bytes) != pin.sha256 {
        bail!("{url} isn't the binary {PIN} names");
    }
    Ok(bytes)
}

fn running_installed() -> Result<bool> {
    let installed =
        file::if_found(fs::canonicalize(BINARY)).with_context(|| format!("reading {BINARY}"))?;
    Ok(installed == Some(env::current_exe()?))
}

#[cfg(test)]
#[path = "../tests/update.rs"]
mod tests;
