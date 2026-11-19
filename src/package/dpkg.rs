use super::{Archive, Backup, Db, Distro, Origin, Package};
use crate::{
    cmd::{self, As},
    fleet::{self, Fleet},
    parallel,
    prompt::Prompt,
    systemd::{self, SYSTEM_DIRS, USER_DIRS},
    walk,
};
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
const EXTENDED_STATES: &str = "/var/lib/apt/extended_states";
const CACHE: &str = "/var/cache/apt/archives";
const UCF_REGISTRY: &str = "/var/lib/ucf/registry";
const UCF_HASHES: &str = "/var/lib/ucf/hashfile";
const ALTERNATIVES: &str = "/var/lib/dpkg/alternatives";
const NO_HASH_YET: &str = "newconffile";
const RETIRED: [&str; 2] = ["obsolete", "remove-on-upgrade"];
const REQUIRED_PRIORITIES: [&str; 2] = ["required", "important"];
const BOOT_FILES: [&str; 1] = ["/usr/bin/sudo"];
const BOOT_FOLDERS: [&str; 7] = [
    "/usr/lib/grub/",
    "/usr/lib/shim/",
    "/usr/lib/systemd/boot/efi/",
    "/usr/lib/firmware/",
    "/usr/share/initramfs-tools/hooks/",
    "/usr/share/initramfs-tools/scripts/",
    "/etc/network/",
];

pub fn load() -> Result<Vec<Package>> {
    let status = fs::read_to_string(STATUS).with_context(|| format!("reading {STATUS}"))?;
    let auto = walk::read_if_any(Path::new(EXTENDED_STATES))?
        .map(|text| auto_installed(&text))
        .transpose()?
        .unwrap_or_default();
    let diversions = walk::read_if_any(Path::new(DIVERSIONS))?
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
        walk::read_if_any(Path::new(UCF_REGISTRY))?,
        walk::read_if_any(Path::new(UCF_HASHES))?,
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
        let Some(target) = walk::if_found(fs::read_link(&master))
            .with_context(|| format!("reading {}", master.display()))?
        else {
            continue;
        };
        let target = resolve.listed(&target)?;
        let links = std::iter::once(master)
            .chain(names.iter().map(|n| Path::new("/etc/alternatives").join(n)));
        claims.extend(links.map(|link| (target.clone(), link)));
    }
    for dir in walk::children(Path::new(fleet::ETC))? {
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
        let is_dir = walk::if_found(fs::symlink_metadata(path))
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
            let resolved = walk::if_found(fs::canonicalize(dir))
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

pub fn install(names: &[&str]) -> Result<()> {
    apt_get(&["update"])?;
    let restore_config = ["-o", "Dpkg::Options::=--force-confmiss"];
    apt_get(&[&["install", "-y"], &restore_config[..], names].concat())
}

pub fn uninstall(db: &Db, names: &[&str]) -> Result<BTreeSet<String>> {
    let before = autoremovable(&[])?;
    cmd::root("apt-mark", &[&["auto"], names].concat())?;
    let after = autoremovable(&[])?;
    let unneeded: BTreeSet<String> = after.difference(&before).cloned().collect();
    remove(db, &unneeded)?;
    Ok(unneeded)
}

fn remove(db: &Db, names: &BTreeSet<String>) -> Result<()> {
    if names.is_empty() {
        return Ok(());
    }
    let list: Vec<&str> = names.iter().map(String::as_str).collect();
    apt_get(&[&["remove", "-y"], &list[..]].concat())?;
    let config = names
        .iter()
        .filter_map(|name| db.resolve(name))
        .flat_map(|package| &package.backups)
        .filter(|backup| backup.origin == Origin::Archive);
    for backup in config {
        if unedited(backup)? {
            fs::remove_file(&backup.path)
                .with_context(|| format!("deleting {}", backup.path.display()))?;
        }
    }
    Ok(())
}

fn unedited(backup: &Backup) -> Result<bool> {
    let bytes = walk::if_found(fs::read(&backup.path))
        .with_context(|| format!("reading {}", backup.path.display()))?;
    Ok(bytes.is_some_and(|bytes| crate::hash::md5(&bytes) == backup.md5))
}

pub fn keep_only(fleet: &Fleet, answer: Option<bool>, prompt: &mut Prompt) -> Result<()> {
    let db = Db::load()?;
    let mut keep = fleet.listed_packages();
    keep.extend(db.needing(&runs_the_machine(&db)?));
    let manual = cmd::stdout(As::Root, "apt-mark", &["showmanual"])?;
    let unmark: Vec<&str> = manual.lines().filter(|n| !keep.contains(*n)).collect();
    if unmark.is_empty() {
        return Ok(());
    }
    let states = tempfile::NamedTempFile::new()?;
    if let Some(text) = walk::read_if_any(Path::new(EXTENDED_STATES))? {
        fs::write(states.path(), text)?;
    }
    let option = format!("Dir::State::extended_states={}", states.path().display());
    let trial = ["-o", option.as_str()];
    cmd::root("apt-mark", &[&trial[..], &["auto"], &unmark[..]].concat())?;
    let unneeded = autoremovable(&trial)?;
    if unneeded.is_empty() {
        return Ok(());
    }
    println!("The fleet's packages don't need these {}:", unneeded.len());
    for name in &unneeded {
        println!("  {name}");
    }
    let confirmed = match answer {
        Some(answer) => answer,
        None => prompt.confirm("Remove them")?,
    };
    if !confirmed {
        return Ok(());
    }
    cmd::root("apt-mark", &[&["auto"], &unmark[..]].concat())?;
    remove(&db, &unneeded)
}

fn runs_the_machine(db: &Db) -> Result<BTreeSet<String>> {
    let release = cmd::stdout(As::Root, "uname", &["-r"])?;
    let mut files: BTreeSet<String> = BOOT_FILES.iter().map(|f| f.to_string()).collect();
    files.insert(format!("/boot/vmlinuz-{}", release.trim()));
    for dirs in [SYSTEM_DIRS, USER_DIRS] {
        let dirs = systemd::paths(&dirs);
        for unit in systemd::enabled(&[&dirs[0]], &dirs)? {
            if let Some(file) = systemd::unit_file(&unit, &dirs) {
                let file = fs::canonicalize(&file)
                    .with_context(|| format!("reading {}", file.display()))?;
                files.insert(file.to_string_lossy().into_owned());
            }
        }
    }
    Ok(needed(db, &files))
}

fn needed(db: &Db, files: &BTreeSet<String>) -> BTreeSet<String> {
    let in_boot_folder = |file: &str| {
        BOOT_FOLDERS
            .iter()
            .any(|dir| file.strip_prefix(dir).is_some_and(|rest| !rest.is_empty()))
    };
    db.packages
        .iter()
        .filter(|package| {
            package.required
                || package
                    .files
                    .iter()
                    .any(|file| files.contains(file) || in_boot_folder(file))
        })
        .map(|package| package.name.clone())
        .collect()
}

fn autoremovable(options: &[&str]) -> Result<BTreeSet<String>> {
    let args = [options, &["--simulate", "autoremove"]].concat();
    let out = cmd::stdout(As::Root, "apt-get", &args)?;
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
