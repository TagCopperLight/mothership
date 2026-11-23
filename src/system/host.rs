use crate::{util::cmd, util::file};
use anyhow::{Context, Result, bail};
use std::{fs, path::Path};

const HOSTS: &str = "/etc/hosts";

pub fn check(name: &str) -> Result<()> {
    let valid = (1..=63).contains(&name.len())
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid {
        bail!("{name} isn't a hostname: use up to 63 of a-z, 0-9 and -, with no - at either end");
    }
    Ok(())
}

pub fn name() -> Result<String> {
    Ok(fs::read_to_string("/etc/hostname")
        .context("reading /etc/hostname")?
        .trim()
        .to_owned())
}

pub fn rename(new: &str) -> Result<()> {
    let old = name()?;
    cmd::root("hostnamectl", &["set-hostname", new])?;
    let hosts = fs::read_to_string(HOSTS).with_context(|| format!("reading {HOSTS}"))?;
    let renamed: Vec<String> = hosts
        .lines()
        .map(|line| match line.strip_prefix("127.0.1.1") {
            Some(names) => {
                let names: Vec<&str> = names
                    .split_whitespace()
                    .map(|name| if name == old { new } else { name })
                    .collect();
                format!("127.0.1.1\t{}", names.join(" "))
            }
            None => line.to_owned(),
        })
        .collect();
    if renamed.join("\n") != hosts.trim_end_matches('\n') {
        file::write(
            Path::new(HOSTS),
            (renamed.join("\n") + "\n").as_bytes(),
            0o644,
        )?;
    }
    Ok(())
}
