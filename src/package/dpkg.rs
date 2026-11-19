use super::{Archive, Backup, Distro, Origin, Package, in_parallel};
use crate::cmd::{self, As};
use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const STATUS: &str = "/var/lib/dpkg/status";
const INFO: &str = "/var/lib/dpkg/info";
const EXTENDED_STATES: &str = "/var/lib/apt/extended_states";
const CACHE: &str = "/var/cache/apt/archives";
const UCF_REGISTRY: &str = "/var/lib/ucf/registry";
const UCF_HASHES: &str = "/var/lib/ucf/hashfile";
const ALTERNATIVES: &str = "/var/lib/dpkg/alternatives";

pub fn load() -> Result<Vec<Package>> {
    let status = fs::read_to_string(STATUS).with_context(|| format!("reading {STATUS}"))?;
    let auto =
        read_if_any(EXTENDED_STATES)?.map_or_else(BTreeSet::new, |text| auto_installed(&text));
    let mut installed: Vec<Stanza> = stanzas(&status)
        .filter(|s| s.field("Status").split_whitespace().nth(2) == Some("installed"))
        .collect();
    installed.sort_by(|a, b| a.field("Package").cmp(b.field("Package")));
    let mut packages = in_parallel(&installed, |stanza| package(stanza, &auto))?;
    claim_written(&mut packages)?;
    Ok(packages)
}

/// Files in /etc that install scripts and dpkg's tools write, which no package lists: config
/// files ucf keeps, the alternatives dpkg picks itself, and the links that start init scripts.
fn claim_written(packages: &mut [Package]) -> Result<()> {
    if let (Some(registry), Some(hashes)) = (read_if_any(UCF_REGISTRY)?, read_if_any(UCF_HASHES)?) {
        for (name, backup) in ucf(&registry, &hashes) {
            if let Some(package) = packages.iter_mut().find(|p| p.name == name) {
                package
                    .files
                    .push(backup.path.to_string_lossy().into_owned());
                package.backups.push(backup);
            }
        }
    }
    let mut claims = Vec::new();
    for entry in fs::read_dir(ALTERNATIVES).with_context(|| format!("reading {ALTERNATIVES}"))? {
        let path = entry?.path();
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let Some(names) = picked_by_dpkg(&text) else {
            continue;
        };
        let master = Path::new("/etc/alternatives").join(path.file_name().expect("read_dir"));
        let Ok(target) = fs::read_link(&master) else {
            continue;
        };
        let links = std::iter::once(master)
            .chain(names.iter().map(|n| Path::new("/etc/alternatives").join(n)));
        claims.extend(links.map(|link| (listed(&target), link)));
    }
    for dir in fs::read_dir("/etc")? {
        let dir = dir?.path();
        let is_rc = dir
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("rc") && n.ends_with(".d"));
        if !is_rc {
            continue;
        }
        for link in fs::read_dir(&dir)? {
            let link = link?.path();
            if let Ok(target) = fs::read_link(&link) {
                claims.push((listed(&dir.join(target)), link));
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

/// Each config file ucf keeps, with the package that registered it and the md5 of what ucf
/// last wrote.
fn ucf(registry: &str, hashes: &str) -> Vec<(String, Backup)> {
    let md5s: HashMap<&str, &str> = hashes
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let md5 = words.next()?;
            Some((words.next()?, md5))
        })
        .collect();
    registry
        .lines()
        .filter_map(|line| {
            let (name, path) = line.split_once('\t')?;
            let path = path.trim();
            Some((
                name.trim().to_owned(),
                Backup {
                    path: PathBuf::from(path),
                    md5: (*md5s.get(path)?).to_owned(),
                    origin: Origin::Script,
                },
            ))
        })
        .collect()
}

/// The names of an alternative's secondary links, when dpkg picks it by priority. One set by
/// hand reads `manual`, and the scan reports it.
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

fn read_if_any(path: &str) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {path}")),
    }
}

fn package(stanza: &Stanza, auto: &BTreeSet<(String, String)>) -> Result<Package> {
    let name = stanza.field("Package").to_owned();
    let arch = stanza.field("Architecture");
    let version = stanza.field("Version");
    // Recommends count, since apt installs them and keeps them installed.
    let depends = ["Pre-Depends", "Depends", "Recommends"]
        .iter()
        .flat_map(|field| names(stanza.field(field)))
        .collect();
    // dpkg writes `newconffile` for a config file it has no hash of yet.
    let backups = stanza
        .field("Conffiles")
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let (path, md5) = (words.next()?, words.next()?);
            (md5 != "newconffile").then(|| Backup {
                path: PathBuf::from(path),
                md5: md5.to_owned(),
                origin: Origin::Archive,
            })
        })
        .collect();
    // A version's epoch keeps its colon in the name of the archive apt downloaded.
    let archive = format!("{name}_{}_{arch}.deb", version.replace(':', "%3a"));
    Ok(Package {
        description: stanza
            .field("Description")
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned(),
        explicit: !is_auto(auto, &name, arch),
        files: files(&name, arch)?,
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

/// Packages of several architectures keep their list under `name:arch`.
fn files(name: &str, arch: &str) -> Result<Vec<String>> {
    let list = [format!("{name}:{arch}.list"), format!("{name}.list")]
        .into_iter()
        .map(|file| Path::new(INFO).join(file))
        .find(|path| path.exists())
        .with_context(|| format!("{INFO} has no file list for {name}"))?;
    let text = fs::read_to_string(&list).with_context(|| format!("reading {}", list.display()))?;
    Ok(text
        .lines()
        .filter(|line| *line != "/.")
        .map(|line| listed(Path::new(line)))
        .collect())
}

/// Resolved like the paths a scan asks about, since some packages still list files under /bin
/// or /lib, which link into /usr. dpkg doesn't mark folders, so the disk says which are.
fn listed(path: &Path) -> String {
    let resolved = path
        .parent()
        .zip(path.file_name())
        .and_then(|(dir, name)| Some(fs::canonicalize(dir).ok()?.join(name)))
        .unwrap_or_else(|| path.to_path_buf());
    let is_dir = fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir());
    let text = resolved.to_string_lossy();
    if is_dir {
        format!("{text}/")
    } else {
        text.into_owned()
    }
}

/// A field's value, its continuation lines included.
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
}

fn stanzas(text: &str) -> impl Iterator<Item = Stanza<'_>> {
    text.split("\n\n")
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            let mut fields: Vec<(&str, String)> = Vec::new();
            for line in s.lines() {
                match (line.strip_prefix(' '), fields.last_mut()) {
                    (Some(more), Some((_, value))) => {
                        value.push('\n');
                        value.push_str(more);
                    }
                    _ => {
                        if let Some((name, value)) = line.split_once(':') {
                            fields.push((name, value.trim().to_owned()));
                        }
                    }
                }
            }
            Stanza { fields }
        })
}

/// `a (>= 1), b:any | c` lists a, b and c: the scan follows whichever is installed.
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

fn auto_installed(text: &str) -> BTreeSet<(String, String)> {
    stanzas(text)
        .filter(|s| s.field("Auto-Installed") == "1")
        .map(|s| {
            (
                s.field("Package").to_owned(),
                s.field("Architecture").to_owned(),
            )
        })
        .collect()
}

/// apt records an `all` package under the machine's own architecture.
fn is_auto(auto: &BTreeSet<(String, String)>, name: &str, arch: &str) -> bool {
    auto.iter()
        .any(|(n, a)| n == name && (a == arch || arch == "all"))
}

/// madison lists only the versions apt's sources offer, where `apt-cache pkgnames` also
/// counts the installed ones.
pub fn cannot_install(name: &str) -> Result<Option<String>> {
    let offered = cmd::stdout(As::Root, "apt-cache", &["madison", name])?;
    Ok(offered
        .trim()
        .is_empty()
        .then(|| "It is in none of apt's sources.".to_owned()))
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

/// apt reads its sources again first, since apply may have just written one.
pub fn install(names: &[&str]) -> Result<()> {
    apt_get(&["update"])?;
    apt_get(&[&["install", "-y"], names].concat())
}

/// What apt's autoremove would remove after marking the packages as dependencies, minus what
/// it would have removed anyway.
pub fn uninstall(names: &[&str]) -> Result<()> {
    let before = autoremovable()?;
    cmd::root("apt-mark", &[&["auto"], names].concat())?;
    let after = autoremovable()?;
    let unneeded: Vec<&str> = after.difference(&before).map(String::as_str).collect();
    if unneeded.is_empty() {
        return Ok(());
    }
    apt_get(&[&["purge", "-y"], &unneeded[..]].concat())
}

fn autoremovable() -> Result<BTreeSet<String>> {
    let out = cmd::stdout(As::Root, "apt-get", &["--simulate", "autoremove"])?;
    Ok(out
        .lines()
        .filter_map(|line| line.strip_prefix("Remv ")?.split_whitespace().next())
        .map(str::to_owned)
        .collect())
}

fn apt_get(args: &[&str]) -> Result<()> {
    let mut command = As::Root.command("apt-get", args);
    command.env("DEBIAN_FRONTEND", "noninteractive");
    cmd::status(command, "apt-get", args)
}

#[cfg(test)]
#[path = "../../tests/dpkg.rs"]
mod tests;
