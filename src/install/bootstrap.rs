use crate::{
    package::Distro,
    util::{
        cmd::{self, As},
        file, http,
    },
};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

const CHEZMOI_VERSION: &str = "2.74.2";
const CHEZMOI_DEBS: [(&str, &str); 2] = [
    (
        "amd64",
        "0493cbc4872a6dfdf4418b5a9b7d72fd8d863459740f64625374e5fce6d5081f",
    ),
    (
        "arm64",
        "32d4b8c2191d7f48e785f4709ecfa1c3f808225f13ca487cdb31b7d81d50d29b",
    ),
];

pub fn git(distro: Distro) -> Result<()> {
    if cmd::found(As::Root, "git") {
        return Ok(());
    }
    distro.bootstrap(&["git"])
}

pub fn ssh(distro: Distro) -> Result<()> {
    if cmd::found(As::Root, "ssh") {
        return Ok(());
    }
    distro.bootstrap(&[distro.ssh_package()])
}

pub fn chezmoi(distro: Distro) -> Result<()> {
    if cmd::found(As::Root, "chezmoi") {
        return Ok(());
    }
    match distro {
        Distro::Arch => distro.bootstrap(&["chezmoi"]),
        Distro::Debian => {
            let dir = tempfile::tempdir()?;
            let deb = download_chezmoi_deb(dir.path())?;
            distro.bootstrap(&[&deb.to_string_lossy()])
        }
    }
}

fn download_chezmoi_deb(dir: &Path) -> Result<PathBuf> {
    let arch = cmd::stdout(As::Root, "dpkg", &["--print-architecture"])?;
    let arch = arch.trim();
    let (_, wanted) = CHEZMOI_DEBS
        .iter()
        .find(|(known, _)| *known == arch)
        .with_context(|| format!("install has no chezmoi .deb for {arch}"))?;
    let name = format!("chezmoi_{CHEZMOI_VERSION}_linux_{arch}.deb");
    let url =
        format!("https://github.com/twpayne/chezmoi/releases/download/v{CHEZMOI_VERSION}/{name}");
    let deb = dir.join(&name);
    let path = deb.to_string_lossy();
    file::write(&deb, &http::download(&url)?, 0o644)?;
    let actual = cmd::stdout(As::Root, "sha256sum", &[&*path])?;
    if actual.split_whitespace().next() != Some(*wanted) {
        bail!("{name} isn't the chezmoi {CHEZMOI_VERSION} install knows");
    }
    Ok(deb)
}
