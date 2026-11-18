use crate::scan::{Item, Kind};
use anyhow::{Context, Result, anyhow};
use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};
use std::{collections::HashMap, fs, io::ErrorKind, path::Path};

/// ignore.txt: `<kind> <glob>` lines.
pub struct Ignore {
    sets: HashMap<Kind, GlobSet>,
}

impl Ignore {
    pub fn load(path: &Path) -> Result<Self> {
        let mut builders: HashMap<Kind, GlobSetBuilder> = HashMap::new();
        for (at, line) in lines(path)? {
            let (kind, pattern) = line
                .split_once(' ')
                .ok_or_else(|| anyhow!("{at}: expected `<kind> <glob>`"))?;
            let kind = Kind::parse(kind).ok_or_else(|| anyhow!("{at}: unknown kind {kind}"))?;
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

    pub fn matches(&self, item: &Item) -> bool {
        self.sets
            .get(&item.kind)
            .is_some_and(|set| set.is_match(&item.subject))
    }
}

/// A list of paths as the scan shows them, one glob per line.
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
        self.0.is_match(subject)
    }
}

/// Each line with its place in the file, leaving out blank lines and comments. A missing file
/// holds none.
fn lines(path: &Path) -> Result<Vec<(String, String)>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    Ok(text
        .lines()
        .enumerate()
        .map(|(number, line)| (format!("{}:{}", path.display(), number + 1), line.trim()))
        .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'))
        .map(|(at, line)| (at, line.to_owned()))
        .collect())
}

/// `*` stays in one folder, `**` crosses folders.
fn glob(pattern: &str, at: &str) -> Result<Glob> {
    GlobBuilder::new(pattern.trim())
        .literal_separator(true)
        .build()
        .with_context(|| at.to_owned())
}
