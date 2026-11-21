use anyhow::{Result, bail};
use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    prefix: bool,
    numbers: Vec<u64>,
    suffix: String,
}

impl Version {
    pub fn parse(text: &str) -> Result<Version> {
        let (prefix, rest) = match text.strip_prefix('v') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let (numbers, suffix) = match rest.split_once('-') {
            Some((numbers, suffix)) if !suffix.is_empty() => (numbers, suffix),
            Some(_) => bail!("{text} isn't a version like 16.4.3"),
            None => (rest, ""),
        };
        let numbers = numbers
            .split('.')
            .map(
                |part| match part.bytes().all(|b| b.is_ascii_digit()) && !part.is_empty() {
                    true => Ok(part.parse::<u64>()?),
                    false => bail!("{text} isn't a version like 16.4.3"),
                },
            )
            .collect::<Result<Vec<u64>>>()?;
        Ok(Version {
            prefix,
            numbers,
            suffix: suffix.to_owned(),
        })
    }

    pub fn major(&self) -> u64 {
        self.numbers[0]
    }

    pub fn same_shape(&self, other: &Version) -> bool {
        self.prefix == other.prefix
            && self.numbers.len() == other.numbers.len()
            && self.suffix == other.suffix
    }

    pub fn follows(&self, newer: &Version) -> bool {
        self.same_shape(newer) && self.major() == newer.major()
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Version) -> Option<Ordering> {
        self.same_shape(other)
            .then(|| self.numbers.cmp(&other.numbers))
    }
}

pub fn check_update(old: &str, new: &str) -> Result<()> {
    let (before, after) = (Version::parse(old)?, Version::parse(new)?);
    if !before.same_shape(&after) {
        bail!("{new} isn't written like {old}");
    }
    if before.major() != after.major() {
        bail!("{new} leaves major version {}", before.major());
    }
    if after < before {
        bail!("{new} is older than {old}");
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/version.rs"]
mod tests;
