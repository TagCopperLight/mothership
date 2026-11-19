use super::{Archive, Backup, Distro, Origin, Package};
use crate::{file, parallel, walk};
use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const STATUS: &str = "/var/lib/dpkg/status";
const INFO: &str = "/var/lib/dpkg/info";
const DIVERSIONS: &str = "/var/lib/dpkg/diversions";
pub(super) const EXTENDED_STATES: &str = "/var/lib/apt/extended_states";
const CACHE: &str = "/var/cache/apt/archives";
const UCF_REGISTRY: &str = "/var/lib/ucf/registry";
const UCF_HASHES: &str = "/var/lib/ucf/hashfile";
const ALTERNATIVES: &str = "/var/lib/dpkg/alternatives";
const NO_HASH_YET: &str = "newconffile";
const RETIRED: [&str; 2] = ["obsolete", "remove-on-upgrade"];
const REQUIRED_PRIORITIES: [&str; 2] = ["required", "important"];

pub fn load() -> Result<Vec<Package>> {
    let status = fs::read_to_string(STATUS).with_context(|| format!("reading {STATUS}"))?;
    let auto = file::read_if_any(Path::new(EXTENDED_STATES))?
        .map(|text| auto_installed(&text))
        .transpose()?
        .unwrap_or_default();
    let diversions = file::read_if_any(Path::new(DIVERSIONS))?
        .map(|text| Diversions::parse(&text))
        .transpose()
        .with_context(|| format!("reading {DIVERSIONS}"))?
        .unwrap_or_default();
    let mut installed = Vec::new();
    for stanza in stanzas(&status).with_context(|| format!("reading {STATUS}"))? {
        if stanza.field("Status").split_whitespace().nth(2) == Some("installed") {
            installed.push(stanza);
        }
    }
    installed.sort_by(|a, b| a.field("Package").cmp(b.field("Package")));
    let mut packages = parallel::map(&installed, |stanza| {
        package(stanza, &auto, &diversions)
            .with_context(|| format!("reading {}'s entry in {STATUS}", stanza.field("Package")))
    })?;
    claim_written(&mut packages)?;
    Ok(packages)
}

fn claim_written(packages: &mut [Package]) -> Result<()> {
    if let (Some(registry), Some(hashes)) = (
        file::read_if_any(Path::new(UCF_REGISTRY))?,
        file::read_if_any(Path::new(UCF_HASHES))?,
    ) {
        for (name, backup) in ucf(&registry, &hashes).context("reading ucf's records")? {
            if let Some(package) = packages.iter_mut().find(|p| p.name == name) {
                package
                    .files
                    .push(backup.path.to_string_lossy().into_owned());
                package.backups.push(backup);
            }
        }
    }
    let mut resolve = Resolve::default();
    let mut claims = Vec::new();
    for path in walk::children(Path::new(ALTERNATIVES))? {
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let Some(names) = picked_by_dpkg(&text) else {
            continue;
        };
        let Some(name) = path.file_name() else {
            continue;
        };
        let master = Path::new("/etc/alternatives").join(name);
        let Some(target) = file::if_found(fs::read_link(&master))
            .with_context(|| format!("reading {}", master.display()))?
        else {
            continue;
        };
        let target = resolve.listed(&target)?;
        let links = std::iter::once(master)
            .chain(names.iter().map(|n| Path::new("/etc/alternatives").join(n)));
        claims.extend(links.map(|link| (target.clone(), link)));
    }
    for dir in walk::children(Path::new(file::ETC))? {
        let is_rc = dir
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("rc") && n.ends_with(".d"));
        if !is_rc {
            continue;
        }
        for link in walk::children(&dir)? {
            if link.is_symlink() {
                let target = fs::read_link(&link)?;
                claims.push((resolve.listed(&dir.join(target))?, link));
            }
        }
    }
    let owner: HashMap<String, usize> = packages
        .iter()
        .enumerate()
        .flat_map(|(i, p)| p.files.iter().map(move |f| (f.clone(), i)))
        .collect();
    for (target, link) in claims {
        if let Some(&i) = owner.get(&target) {
            packages[i].files.push(link.to_string_lossy().into_owned());
        }
    }
    Ok(())
}

fn ucf(registry: &str, hashes: &str) -> Result<Vec<(String, Backup)>> {
    let md5s: HashMap<&str, &str> = hashes
        .lines()
        .map(|line| {
            let (md5, path) = line
                .split_once(' ')
                .with_context(|| format!("{UCF_HASHES} has a line without a path: {line}"))?;
            Ok((path.trim_start(), md5))
        })
        .collect::<Result<_>>()?;
    let mut found = Vec::new();
    for line in registry.lines() {
        let (name, path) = line
            .split_once('\t')
            .with_context(|| format!("{UCF_REGISTRY} has a line without a path: {line}"))?;
        let path = path.trim();
        if let Some(md5) = md5s.get(path) {
            found.push((
                name.trim().to_owned(),
                Backup {
                    path: PathBuf::from(path),
                    md5: (*md5).to_owned(),
                    origin: Origin::Script,
                },
            ));
        }
    }
    Ok(found)
}

fn picked_by_dpkg(state: &str) -> Option<Vec<&str>> {
    let mut lines = state.lines();
    if lines.next()? != "auto" {
        return None;
    }
    lines.next()?;
    Some(
        lines
            .take_while(|line| !line.is_empty())
            .step_by(2)
            .collect(),
    )
}

#[derive(Default)]
struct Diversions(HashMap<String, Diversion>);

struct Diversion {
    to: String,
    by: Option<String>,
}

impl Diversions {
    fn parse(text: &str) -> Result<Self> {
        let lines: Vec<&str> = text.lines().collect();
        let (groups, rest) = lines.as_chunks::<3>();
        if !rest.is_empty() {
            bail!("its lines don't come in threes");
        }
        Ok(Diversions(
            groups
                .iter()
                .map(|[from, to, by]| {
                    let diversion = Diversion {
                        to: (*to).to_owned(),
                        by: (*by != ":").then(|| (*by).to_owned()),
                    };
                    ((*from).to_owned(), diversion)
                })
                .collect(),
        ))
    }

    fn of<'a>(&'a self, path: &'a str, package: &str) -> &'a str {
        match self.0.get(path) {
            Some(diversion) if diversion.by.as_deref() != Some(package) => &diversion.to,
            _ => path,
        }
    }
}

fn package(
    stanza: &Stanza,
    auto: &BTreeSet<(String, String)>,
    diversions: &Diversions,
) -> Result<Package> {
    let name = stanza.required("Package")?.to_owned();
    let arch = stanza.required("Architecture")?;
    let version = stanza.required("Version")?;
    let depends = ["Pre-Depends", "Depends", "Recommends"]
        .iter()
        .flat_map(|field| names(stanza.field(field)))
        .collect();
    let mut backups = Vec::new();
    for line in stanza.field("Conffiles").lines().filter(|l| !l.is_empty()) {
        if let Some((path, md5)) = conffile(line)? {
            backups.push(Backup {
                path: PathBuf::from(diversions.of(path, &name)),
                md5: md5.to_owned(),
                origin: Origin::Archive,
            });
        }
    }
    let archive = format!("{name}_{}_{arch}.deb", version.replace(':', "%3a"));
    Ok(Package {
        description: stanza
            .field("Description")
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned(),
        explicit: !is_auto(auto, &name, arch),
        required: REQUIRED_PRIORITIES.contains(&stanza.field("Priority"))
            || stanza.field("Essential") == "yes",
        files: files(&name, arch, diversions)?,
        backups,
        depends,
        provides: names(stanza.field("Provides")),
        archive: Archive {
            path: Path::new(CACHE).join(archive),
            distro: Distro::Debian,
        },
        name,
    })
}

fn conffile(line: &str) -> Result<Option<(&str, &str)>> {
    let (rest, last) = split_conffile(line, line)?;
    let (path, md5, retired) = if RETIRED.contains(&last) {
        let (path, md5) = split_conffile(rest, line)?;
        (path, md5, true)
    } else {
        (rest, last, false)
    };
    Ok((!retired && md5 != NO_HASH_YET).then_some((path, md5)))
}

fn split_conffile<'a>(text: &'a str, line: &str) -> Result<(&'a str, &'a str)> {
    text.rsplit_once(' ')
        .filter(|(path, _)| path.starts_with('/'))
        .with_context(|| format!("a Conffiles line is malformed: {line}"))
}

fn files(name: &str, arch: &str, diversions: &Diversions) -> Result<Vec<String>> {
    let list = [format!("{name}:{arch}.list"), format!("{name}.list")]
        .into_iter()
        .map(|file| Path::new(INFO).join(file))
        .find(|path| path.exists())
        .with_context(|| format!("{INFO} has no file list for {name}"))?;
    let text = fs::read_to_string(&list).with_context(|| format!("reading {}", list.display()))?;
    let mut resolve = Resolve::default();
    text.lines()
        .filter(|line| *line != "/.")
        .map(|line| resolve.listed(Path::new(diversions.of(line, name))))
        .collect()
}

#[derive(Default)]
struct Resolve {
    dirs: HashMap<PathBuf, PathBuf>,
}

impl Resolve {
    fn listed(&mut self, path: &Path) -> Result<String> {
        let resolved = match path.parent().zip(path.file_name()) {
            Some((dir, name)) => self.dir(dir)?.join(name),
            None => path.to_path_buf(),
        };
        let is_dir = file::if_found(fs::symlink_metadata(path))
            .with_context(|| format!("reading {}", path.display()))?
            .is_some_and(|meta| meta.is_dir());
        let text = resolved.to_string_lossy();
        Ok(if is_dir {
            format!("{text}/")
        } else {
            text.into_owned()
        })
    }

    fn dir(&mut self, dir: &Path) -> Result<&PathBuf> {
        if !self.dirs.contains_key(dir) {
            let resolved = file::if_found(fs::canonicalize(dir))
                .with_context(|| format!("reading {}", dir.display()))?
                .unwrap_or_else(|| dir.to_path_buf());
            self.dirs.insert(dir.to_path_buf(), resolved);
        }
        Ok(&self.dirs[dir])
    }
}

struct Stanza<'a> {
    fields: Vec<(&'a str, String)>,
}

impl Stanza<'_> {
    fn field(&self, name: &str) -> &str {
        self.fields
            .iter()
            .find(|(field, _)| *field == name)
            .map_or("", |(_, value)| value)
    }

    fn required(&self, name: &str) -> Result<&str> {
        let value = self.field(name);
        if value.is_empty() {
            bail!("an entry has no {name}");
        }
        Ok(value)
    }
}

fn stanzas(text: &str) -> Result<Vec<Stanza<'_>>> {
    let mut found = Vec::new();
    for block in text.split("\n\n").filter(|s| !s.trim().is_empty()) {
        let mut fields: Vec<(&str, String)> = Vec::new();
        for line in block.lines() {
            match (line.strip_prefix(' '), fields.last_mut()) {
                (Some(more), Some((_, value))) => {
                    value.push('\n');
                    value.push_str(more);
                }
                _ => {
                    let (name, value) = line.split_once(':').with_context(|| {
                        format!("a line is neither a field nor more of one: {line}")
                    })?;
                    fields.push((name, value.trim().to_owned()));
                }
            }
        }
        found.push(Stanza { fields });
    }
    Ok(found)
}

fn names(field: &str) -> Vec<String> {
    field
        .split([',', '|'])
        .filter_map(|dep| {
            let name = dep.split_whitespace().next()?;
            let name = name.split(['(', ':']).next()?;
            (!name.is_empty()).then(|| name.to_owned())
        })
        .collect()
}

fn auto_installed(text: &str) -> Result<BTreeSet<(String, String)>> {
    Ok(stanzas(text)
        .with_context(|| format!("reading {EXTENDED_STATES}"))?
        .iter()
        .filter(|s| s.field("Auto-Installed") == "1")
        .map(|s| {
            (
                s.field("Package").to_owned(),
                s.field("Architecture").to_owned(),
            )
        })
        .collect())
}

fn is_auto(auto: &BTreeSet<(String, String)>, name: &str, arch: &str) -> bool {
    auto.iter()
        .any(|(n, a)| n == name && (a == arch || arch == "all"))
}

pub fn shipped(archive: &Path, member: &Path) -> Result<Vec<u8>> {
    let mut deb = Command::new("dpkg-deb")
        .arg("--fsys-tarfile")
        .arg(archive)
        .stdout(Stdio::piped())
        .spawn()
        .context("running dpkg-deb")?;
    let tar = Command::new("tar")
        .arg("-xOf")
        .arg("-")
        .arg(Path::new(".").join(member))
        .stdin(deb.stdout.take().expect("piped above"))
        .output()
        .context("running tar")?;
    if !deb.wait()?.success() {
        bail!("dpkg-deb --fsys-tarfile {} failed", archive.display());
    }
    if !tar.status.success() {
        bail!(
            "tar couldn't find {} in {}: {}",
            member.display(),
            archive.display(),
            String::from_utf8_lossy(&tar.stderr).trim()
        );
    }
    Ok(tar.stdout)
}

#[cfg(test)]
#[path = "../../tests/dpkg.rs"]
mod tests;
