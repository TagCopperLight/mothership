use crate::{
    cmd::{self, As},
    file, fleet, hash,
};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{
    collections::BTreeMap, env, fs, os::unix::process::CommandExt, path::Path, process::Command,
};

pub const BINARY: &str = "/usr/local/bin/mothership";
const PIN: &str = "mothership.toml";
const RELEASES: &str = "https://github.com/TagCopperLight/mothership/releases/download";

#[derive(Deserialize)]
struct Pin {
    version: String,
    sha256: BTreeMap<String, String>,
}

impl Pin {
    fn sha256(&self) -> Result<&str> {
        let arch = env::consts::ARCH;
        self.sha256
            .get(arch)
            .map(String::as_str)
            .with_context(|| format!("the fleet's {PIN} names no mothership binary for {arch}"))
    }
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
    let wanted = pin.sha256()?;
    if installed.as_deref() == Some(wanted) {
        return Ok(Outcome::Current);
    }
    println!("The fleet runs mothership {}.", pin.version);
    if !approve()? {
        return Ok(Outcome::Declined);
    }
    let restart = running_installed()?;
    file::write(Path::new(BINARY), &download(&pin.version, wanted)?, 0o755)?;
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

fn download(version: &str, sha256: &str) -> Result<Vec<u8>> {
    let url = format!("{RELEASES}/v{version}/mothership-{}", env::consts::ARCH);
    let bytes = cmd::bytes(As::Root, "curl", &["-fsSL", &url])?;
    if hash::sha256(&bytes) != sha256 {
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
