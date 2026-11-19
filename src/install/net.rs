use crate::{file, package::Distro, walk};
use anyhow::{Context, Result, bail};
use std::{
    net::{TcpStream, ToSocketAddrs},
    path::Path,
    thread,
    time::Duration,
};

const PAUSE: Duration = Duration::from_secs(10);
const MIRRORLIST: &str = "/etc/pacman.d/mirrorlist";
const APT_SOURCES: &str = "/etc/apt/sources.list";
const APT_SOURCES_DIR: &str = "/etc/apt/sources.list.d";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scheme {
    Http,
    Https,
    Ssh,
    Git,
}

impl Scheme {
    fn parse(name: &str) -> Option<Scheme> {
        match name {
            "http" => Some(Scheme::Http),
            "https" => Some(Scheme::Https),
            "ssh" | "git+ssh" | "ssh+git" => Some(Scheme::Ssh),
            "git" => Some(Scheme::Git),
            _ => None,
        }
    }

    fn port(self) -> u16 {
        match self {
            Scheme::Http => 80,
            Scheme::Https => 443,
            Scheme::Ssh => 22,
            Scheme::Git => 9418,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Remote<'a> {
    pub scheme: Scheme,
    pub userinfo: Option<&'a str>,
    pub host: &'a str,
    pub port: u16,
}

pub fn remote(url: &str) -> Result<Option<Remote<'_>>> {
    if let Some((scheme, rest)) = url.split_once("://") {
        if scheme == "file" {
            return Ok(None);
        }
        let scheme = Scheme::parse(scheme).with_context(|| {
            format!("install can't reach {url}: it knows http, https, ssh and git")
        })?;
        let authority = rest.split('/').next().unwrap_or_default();
        let (userinfo, host) = split_userinfo(authority);
        let (host, port) = match host.rsplit_once(':') {
            Some((host, port)) if !port.contains(']') => (
                host,
                port.parse()
                    .with_context(|| format!("{url} has no port number after :"))?,
            ),
            _ => (host, scheme.port()),
        };
        if host.is_empty() {
            bail!("{url} names no host");
        }
        return Ok(Some(Remote {
            scheme,
            userinfo,
            host,
            port,
        }));
    }
    match url.split_once(':') {
        Some((before, _)) if !before.contains('/') => {
            let (userinfo, host) = split_userinfo(before);
            Ok(Some(Remote {
                scheme: Scheme::Ssh,
                userinfo,
                host,
                port: Scheme::Ssh.port(),
            }))
        }
        _ => Ok(None),
    }
}

fn split_userinfo(authority: &str) -> (Option<&str>, &str) {
    match authority.rsplit_once('@') {
        Some((userinfo, host)) => (Some(userinfo), host),
        None => (None, authority),
    }
}

pub struct Needs(Option<String>);

impl Needs {
    pub fn url(url: &str) -> Result<Needs> {
        Ok(Needs(
            remote(url)?.map(|r| format!("{}:{}", r.host, r.port)),
        ))
    }

    pub fn mirror(distro: Distro) -> Result<Needs> {
        let first = match distro {
            Distro::Arch => first_pacman_server()?,
            Distro::Debian => first_apt_uri()?,
        };
        match first {
            Some(url) if url.starts_with("http://") || url.starts_with("https://") => {
                Needs::url(&url)
            }
            _ => Ok(Needs(None)),
        }
    }
}

fn first_pacman_server() -> Result<Option<String>> {
    let text = file::read_if_any(Path::new(MIRRORLIST))?.unwrap_or_default();
    Ok(text.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "Server").then(|| value.trim().to_owned())
    }))
}

fn first_apt_uri() -> Result<Option<String>> {
    let dir = Path::new(APT_SOURCES_DIR);
    let mut files = vec![Path::new(APT_SOURCES).to_path_buf()];
    if dir.exists() {
        files.extend(walk::files(dir)?.into_iter().map(|f| dir.join(f)));
    }
    for file in files {
        let Some(text) = file::read_if_any(&file)? else {
            continue;
        };
        if let Some(uri) = text.lines().find_map(apt_uri) {
            return Ok(Some(uri.to_owned()));
        }
    }
    Ok(None)
}

fn apt_uri(line: &str) -> Option<&str> {
    if let Some(uris) = line.strip_prefix("URIs:") {
        return uris.split_whitespace().next();
    }
    let rest = line.strip_prefix("deb ")?.trim_start();
    let rest = match rest.strip_prefix('[') {
        Some(options) => options.split_once(']')?.1,
        None => rest,
    };
    rest.split_whitespace().next()
}

fn reachable(address: &str) -> bool {
    let Ok(addresses) = address.to_socket_addrs() else {
        return false;
    };
    addresses
        .into_iter()
        .any(|a| TcpStream::connect_timeout(&a, Duration::from_secs(5)).is_ok())
}

pub fn retry<T>(what: &str, needs: &Needs, mut step: impl FnMut() -> Result<T>) -> Result<T> {
    let Some(address) = &needs.0 else {
        return step();
    };
    loop {
        let error = match step() {
            Err(e) if !reachable(address) => e,
            result => return result,
        };
        println!("{what} failed, and {address} doesn't answer: {error:#}");
        println!("Waiting for the network…");
        while !reachable(address) {
            thread::sleep(PAUSE);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/net.rs"]
mod tests;
