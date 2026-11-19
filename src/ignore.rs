use crate::{
    file,
    kind::{Kind, Level},
};
use anyhow::{Context, Result, anyhow, bail};
use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};
use std::{collections::HashMap, path::Path};

#[derive(Clone)]
pub struct Ignore {
    sets: HashMap<Kind, GlobSet>,
}

impl Ignore {
    pub fn load(path: &Path, level: Level) -> Result<Self> {
        let mut builders: HashMap<Kind, GlobSetBuilder> = HashMap::new();
        for (at, line) in lines(path)? {
            let (kind, pattern) = line
                .split_once(' ')
                .ok_or_else(|| anyhow!("{at}: expected `<kind> <glob>`"))?;
            let kind: Kind = kind
                .parse()
                .map_err(|_| anyhow!("{at}: unknown kind {kind}"))?;
            if kind.level() != level {
                let other = match level {
                    Level::Machine => "a user's",
                    Level::User => "the type's",
                };
                bail!("{at}: {} lines go in {other} ignore.txt", kind.name());
            }
            builders
                .entry(kind)
                .or_insert_with(GlobSetBuilder::new)
                .add(glob(pattern, &at)?);
        }
        let sets = builders
            .into_iter()
            .map(|(kind, builder)| Ok((kind, builder.build()?)))
            .collect::<Result<_>>()?;
        Ok(Ignore { sets })
    }

    pub fn matches(&self, kind: Kind, subject: &str) -> bool {
        self.sets
            .get(&kind)
            .is_some_and(|set| in_matching_folder(set, subject))
    }
}

#[derive(Clone)]
pub struct Paths(GlobSet);

impl Paths {
    pub fn load(path: &Path) -> Result<Self> {
        let mut builder = GlobSetBuilder::new();
        for (at, line) in lines(path)? {
            builder.add(glob(&line, &at)?);
        }
        Ok(Paths(builder.build()?))
    }

    pub fn matches(&self, subject: &str) -> bool {
        in_matching_folder(&self.0, subject)
    }
}

fn in_matching_folder(set: &GlobSet, subject: &str) -> bool {
    Path::new(subject)
        .ancestors()
        .filter(|path| !path.as_os_str().is_empty())
        .any(|path| set.is_match(path))
}

fn lines(path: &Path) -> Result<Vec<(String, String)>> {
    let text = file::read_if_any(path)?.unwrap_or_default();
    Ok(text
        .lines()
        .enumerate()
        .map(|(number, line)| (format!("{}:{}", path.display(), number + 1), line.trim()))
        .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'))
        .map(|(at, line)| (at, line.to_owned()))
        .collect())
}

fn glob(pattern: &str, at: &str) -> Result<Glob> {
    GlobBuilder::new(pattern.trim())
        .literal_separator(true)
        .build()
        .with_context(|| at.to_owned())
}

#[cfg(test)]
#[path = "../tests/ignore.rs"]
mod tests;
