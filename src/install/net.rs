use anyhow::{Context, Result, bail};

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

#[cfg(test)]
#[path = "../../tests/install/net.rs"]
mod tests;
