use crate::scan::{Item, Kind};
use anyhow::{Context, Result, anyhow};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use std::{collections::HashMap, fs, io::ErrorKind, path::Path};

/// `types/<type>/ignore.txt`: one `<kind> <glob>` per line, matched against what the scan reports.
pub struct Ignore {
    sets: HashMap<Kind, GlobSet>,
}

impl Ignore {
    pub fn load(path: &Path) -> Result<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let mut builders: HashMap<Kind, GlobSetBuilder> = HashMap::new();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let at = || format!("{}:{}", path.display(), number + 1);
            let (kind, pattern) = line
                .split_once(' ')
                .ok_or_else(|| anyhow!("{}: expected `<kind> <glob>`", at()))?;
            let kind = Kind::parse(kind).ok_or_else(|| anyhow!("{}: unknown kind {kind}", at()))?;
            let glob = GlobBuilder::new(pattern.trim())
                .literal_separator(true)
                .build()
                .with_context(at)?;
            builders
                .entry(kind)
                .or_insert_with(GlobSetBuilder::new)
                .add(glob);
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
