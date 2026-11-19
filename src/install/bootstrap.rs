use super::net::{self, Needs};
use crate::{
    cmd::{self, As},
    package::Distro,
};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

const CHEZMOI_VERSION: &str = "2.74.1";
const CHEZMOI_DEBS: [(&str, &str); 2] = [
    (
        "amd64",
        "d63f7a876066f8de992c5a47046049ad569e05d8ab30d8507f123281d7cea15c",
    ),
    (
        "arm64",
        "a348c5fa9d51cebb8a38db0711df4658a1d6ad79aa2667c4b58b5f4b08630a4d",
    ),
];

pub fn git(distro: Distro) -> Result<()> {
    if cmd::found(As::Root, "git") {
        return Ok(());
    }
    net::retry("Installing git", &Needs::mirror(distro)?, || {
        distro.bootstrap(&["git"])
    })
}

pub fn gh(distro: Distro) -> Result<()> {
    if cmd::found(As::Root, "gh") {
        return Ok(());
    }
    net::retry("Installing gh", &Needs::mirror(distro)?, || {
        distro.bootstrap(&[distro.gh_package()])
    })
}

pub fn chezmoi(distro: Distro) -> Result<()> {
    if cmd::found(As::Root, "chezmoi") {
        return Ok(());
    }
    match distro {
        Distro::Arch => net::retry("Installing chezmoi", &Needs::mirror(distro)?, || {
            distro.bootstrap(&["chezmoi"])
        }),
        Distro::Debian => {
            let dir = tempfile::tempdir()?;
            let deb = download_chezmoi_deb(dir.path())?;
            net::retry("Installing chezmoi", &Needs::mirror(distro)?, || {
                distro.bootstrap(&[&deb.to_string_lossy()])
            })
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
    net::retry("Downloading chezmoi", &Needs::url(&url)?, || {
        cmd::root("curl", &["-fsSL", "-o", &path, &url])
    })?;
    let actual = cmd::stdout(As::Root, "sha256sum", &[&path])?;
    if actual.split_whitespace().next() != Some(*wanted) {
        bail!("{name} isn't the chezmoi {CHEZMOI_VERSION} install knows");
    }
    Ok(deb)
}
