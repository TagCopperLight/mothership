pub mod github;
pub mod recovery;
pub mod sign;
pub mod ssh;
pub mod update;

use crate::{
    fleet::{
        self, Fleet, Machine, Repo, checked,
        homes::{self, Home, Reader},
        machines::{self, Entry, Machines, probe_name, same_key},
    },
    home::chezmoi,
    probe,
    secrets::{
        AGE, Keys, ROOT_KEY, Recipient, Storage, encrypt_for, key_path, make_key, public, root_key,
        user_key,
    },
    services,
    system::user::User,
    util::file,
    util::prompt::Prompt,
    util::style,
    util::walk,
};
use age::x25519;
use anyhow::{Context, Result, anyhow, bail};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Age,
    Ssh,
    Signing,
}

pub struct MachineKey {
    pub name: String,
    pub sort: Sort,
    pub public: String,
    pub target: String,
    pub trusted: bool,
}

pub const SIGNING: &str = "signing";
const PROBE: &str = "probe";
const PROBE_SSH: &str = "probe-ssh";
const ROOT: &str = "root";
const OPTION: &str = "--option";

fn owners(fleet: &Fleet) -> Vec<(String, Option<&User>, PathBuf)> {
    let mut owners = vec![(ROOT.to_owned(), None, fleet.type_dir.clone())];
    for member in &fleet.members {
        owners.push((member.name.clone(), Some(member.user()), member.dir.clone()));
    }
    owners
}

pub fn create_keys(fleet: &Fleet) -> Result<()> {
    if sign::machine_key()?.is_none() {
        sign::create_machine_key()?;
    }
    if probe::unit::runs_here(fleet) {
        for name in [probe::unit::SIGNING, probe::unit::SSH] {
            let path = probe::unit::key_path(name);
            if sign::read_key(&path)?.is_none() {
                sign::create_key(&path)?;
            }
        }
    }
    for (_, user, _) in owners(fleet) {
        match user {
            None if root_key()?.is_none() => drop(make_key(Path::new(ROOT_KEY))?),
            None => {}
            Some(user) => {
                if user_key(user)?.is_none() {
                    user.with_rights(|| make_key(&key_path(user)))?;
                }
                if ssh::user_key(user)?.is_none() {
                    ssh::create_key(user)?;
                }
            }
        }
    }
    Ok(())
}

pub fn machine_keys(fleet: &Fleet) -> Result<Vec<MachineKey>> {
    let entry = fleet.machines.get(&fleet.name);
    let listed = |user: Option<&User>, pick: fn(&machines::UserKeys) -> &Option<String>| {
        entry.and_then(|entry| match user {
            None => entry.keys.age.as_deref(),
            Some(user) => pick(entry.users.get(&user.name)?).as_deref(),
        })
    };
    let mut keys = Vec::new();
    for (name, user, _) in owners(fleet) {
        let target = format!("{}/{name}", fleet.type_name);
        let age = match user {
            None => root_key()?,
            Some(user) => user_key(user)?,
        };
        if let Some(key) = age {
            let public = public(&key);
            keys.push(MachineKey {
                name: name.clone(),
                sort: Sort::Age,
                trusted: same_key(listed(user, |keys| &keys.age), &public),
                public,
                target: target.clone(),
            });
        }
        if let Some(user) = user
            && let Some(public) = ssh::user_key(user)?
        {
            keys.push(MachineKey {
                name,
                sort: Sort::Ssh,
                trusted: same_key(listed(Some(user), |keys| &keys.ssh), &public),
                public,
                target,
            });
        }
    }
    let file = machines::path(Path::new(""), &fleet.name)
        .display()
        .to_string();
    let root_key = |pick: fn(&machines::RootKeys) -> &Option<String>| {
        entry.and_then(|entry| pick(&entry.keys).as_deref())
    };
    if let Some(key) = sign::machine_key()? {
        let public = sign::public(&key)?;
        keys.push(MachineKey {
            name: SIGNING.to_owned(),
            sort: Sort::Signing,
            trusted: same_key(root_key(|keys| &keys.signing), &public),
            public,
            target: file.clone(),
        });
    }
    if probe::unit::runs_here(fleet)
        && let Some(signing) = sign::read_key(&probe::unit::key_path(probe::unit::SIGNING))?
        && let Some(ssh) = sign::read_key(&probe::unit::key_path(probe::unit::SSH))?
    {
        let trusted = same_key(root_key(|keys| &keys.probe), &sign::public(&signing)?);
        for (name, sort, key, target) in [
            (PROBE, Sort::Signing, &signing, file.as_str()),
            (PROBE_SSH, Sort::Ssh, &ssh, "GitHub"),
        ] {
            keys.push(MachineKey {
                name: name.to_owned(),
                sort,
                public: sign::public(key)?,
                target: target.to_owned(),
                trusted,
            });
        }
    }
    Ok(keys)
}

pub fn pending(fleet: &Fleet) -> Result<Vec<String>> {
    Ok(machine_keys(fleet)?
        .into_iter()
        .filter(|key| !key.trusted)
        .map(|key| format!("{}={}", key.name, key.public))
        .collect())
}

pub fn trust_command(fleet: &Fleet) -> Result<Option<String>> {
    let entries = pending(fleet)?;
    let quoted: Vec<String> = entries
        .iter()
        .map(|entry| match entry.contains(' ') {
            true => format!("'{entry}'"),
            false => entry.clone(),
        })
        .collect();
    let options: String = fleet
        .machine
        .options
        .iter()
        .map(|option| format!(" {OPTION} {option}"))
        .collect();
    Ok((!entries.is_empty()).then(|| {
        format!(
            "sudo mothership trust {} {}{options} {}",
            fleet.name,
            fleet.machine.device_type,
            quoted.join(" ")
        )
    }))
}

pub fn options_and_keys(args: &[String]) -> Result<(Vec<String>, &[String])> {
    let mut options = Vec::new();
    let mut rest = args;
    while let [flag, option, after @ ..] = rest
        && flag == OPTION
    {
        if options.contains(option) {
            bail!("{OPTION} {option} is given twice");
        }
        options.push(option.clone());
        rest = after;
    }
    Ok((options, rest))
}

pub fn trust_host(
    fleet: &Fleet,
    keys: &Keys,
    host: &str,
    machine: &Machine,
    entries: &[String],
    prompt: &mut Prompt,
) -> Result<Vec<github::Key>> {
    let file = machines::path(Path::new(""), host);
    let mut entry = match fleet.machines.get(host) {
        Some(known)
            if known.device_type != machine.device_type || known.options != machine.options =>
        {
            bail!(
                "{} says {host} is a {} with options [{}]: change it there",
                file.display(),
                known.device_type,
                known.options.join(", ")
            )
        }
        Some(known) => known.clone(),
        None => Entry::new(&machine.device_type, &machine.options),
    };
    let mut uploads = Vec::new();
    let mut done = Vec::new();
    for pair in entries {
        let (name, key) = pair
            .split_once('=')
            .with_context(|| format!("{pair} isn't <name>=<key>"))?;
        let lists = |what: &str| format!("{} lists another {what} for {host}", file.display());
        match name {
            PROBE => {
                if set(&mut entry.keys.probe, ssh::normalize(key)?, || {
                    lists("probe key")
                })? {
                    done.push(format!(
                        "{} changes the versions of the fleet's images now.",
                        probe_name(host)
                    ));
                }
            }
            PROBE_SSH => uploads.push(github::Key {
                title: github::title(host, PROBE),
                public: ssh::normalize(key)?,
                kind: github::Use::Deploy,
            }),
            SIGNING => {
                let public = ssh::normalize(key)?;
                if set(&mut entry.keys.signing, public.clone(), || {
                    lists("signing key")
                })? {
                    entry.signs = Some(ask_reach(fleet, host, &machine.device_type, prompt)?);
                    done.push(format!("{host} signs for the fleet now."));
                }
                uploads.push(github::Key {
                    title: host.to_owned(),
                    public,
                    kind: github::Use::Signing,
                });
            }
            ROOT if key.starts_with("ssh-") => {
                bail!("root has no SSH key in the fleet, only its users do")
            }
            ROOT => {
                age_key(key)?;
                set(&mut entry.keys.age, key.to_owned(), || {
                    lists("age key of root")
                })?;
            }
            user if key.starts_with("ssh-") => {
                let public = ssh::normalize(key)?;
                let slot = &mut entry.users.entry(user.to_owned()).or_default().ssh;
                if set(slot, public.clone(), || {
                    lists(&format!("SSH key of {user}"))
                })? {
                    done.push(format!(
                        "{}/{user} on {host} has an SSH key in the fleet now.",
                        machine.device_type
                    ));
                }
                uploads.push(github::Key {
                    title: github::title(host, user),
                    public,
                    kind: github::Use::Login,
                });
            }
            user => {
                age_key(key)?;
                let slot = &mut entry.users.entry(user.to_owned()).or_default().age;
                set(slot, key.to_owned(), || {
                    lists(&format!("age key of {user}"))
                })?;
            }
        }
    }
    let mut after = fleet.machines.clone();
    after.insert(host, entry.clone());
    after.check(&fleet.dir)?;
    Machines::check_options(&fleet.dir, &entry)?;
    let changed = changed_readers(fleet, &after)?;
    reseal(fleet, keys, &after, &changed, &mut Readers::new(prompt))?;
    after.write(&fleet.dir, host)?;
    for reader in &changed {
        let count = secrets_of(fleet, reader)?.len();
        println!("{host} opens the {count} secrets of {reader} now.");
    }
    for line in done {
        println!("{line}");
    }
    Ok(uploads)
}

fn set(slot: &mut Option<String>, key: String, taken: impl Fn() -> String) -> Result<bool> {
    match slot {
        None => {
            *slot = Some(key);
            Ok(true)
        }
        Some(known) if same_key(Some(known), &key) => Ok(false),
        Some(_) => bail!("{}: retire it first", taken()),
    }
}

fn age_key(key: &str) -> Result<()> {
    key.parse::<x25519::Recipient>()
        .map(drop)
        .map_err(|e| anyhow!("{key}: {e}"))
}

fn ask_reach(
    fleet: &Fleet,
    host: &str,
    type_name: &str,
    prompt: &mut Prompt,
) -> Result<sign::Reach> {
    let own = format!("{}/{type_name}", fleet::TYPES);
    let question = format!(
        "{host} may change: Enter for {own}, * for everything, or folders separated by spaces ›"
    );
    loop {
        let answer = prompt.line_or_empty(&question)?;
        let words: Vec<&str> = match answer.as_str() {
            "" => vec![own.as_str()],
            "*" => Vec::new(),
            folders => folders.split_whitespace().collect(),
        };
        match reach(fleet, &words) {
            Ok(reach) => return Ok(reach),
            Err(e) => println!("  {e:#}"),
        }
    }
}

fn reach(fleet: &Fleet, folders: &[&str]) -> Result<sign::Reach> {
    let reach = sign::Reach::parse(folders)?;
    if let sign::Reach::Folders(folders) = &reach {
        for folder in folders {
            if !fleet.dir.join(folder).is_dir() {
                bail!("the fleet has no folder {}", folder.display());
            }
        }
    }
    if reach == sign::Reach::Versions {
        bail!("only the probe signs versions alone");
    }
    Ok(reach)
}

fn changed_readers(fleet: &Fleet, after: &Machines) -> Result<Vec<Reader>> {
    Ok(homes::readers(&fleet.dir)?
        .into_iter()
        .filter(|reader| fleet.machines.recipients(reader) != after.recipients(reader))
        .collect())
}

pub fn retire_host(fleet: &Fleet, host: &str) -> Result<()> {
    let mut after = fleet.machines.clone();
    let entry = after
        .remove(host)
        .with_context(|| format!("the fleet has no machine {host}"))?;
    let changed = changed_readers(fleet, &after)?;
    after.write(&fleet.dir, host)?;
    for (name, key) in [
        (host.to_owned(), &entry.keys.signing),
        (probe_name(host), &entry.keys.probe),
    ] {
        if key.is_some() {
            println!("{name} signs nothing now.");
        }
    }
    for reader in &changed {
        let secrets = secrets_of(fleet, reader)?;
        if secrets.is_empty() {
            continue;
        }
        println!("{host} can still open these secrets of {reader}, so change them:");
        for secret in &secrets {
            println!("  {}", secret.shown);
        }
    }
    fleet::check_logins(&fleet.dir).with_context(|| format!("{host} stays"))
}

pub fn reseal_everything(fleet: &Fleet, keys: &Keys, prompt: &mut Prompt) -> Result<()> {
    let all = homes::readers(&fleet.dir)?;
    reseal(
        fleet,
        keys,
        &fleet.machines,
        &all,
        &mut Readers::new(prompt),
    )
}

fn reader(fleet: &Fleet, text: &str) -> Result<Reader> {
    let (type_name, user) = text
        .split_once('/')
        .with_context(|| format!("{text} is not <type>/root or <type>/<user>"))?;
    let type_dir = fleet::type_dir(&fleet.dir, type_name);
    if type_name.is_empty() || type_name.contains("..") || !type_dir.is_dir() {
        bail!("the fleet has no type {type_name}");
    }
    if user == ROOT {
        return Ok(Reader::Root(type_name.to_owned()));
    }
    let home = Home {
        type_name: type_name.to_owned(),
        user: user.to_owned(),
    };
    let dir = home.dir(&fleet.dir);
    if user.is_empty() || user.contains('/') || user.contains("..") || !dir.is_dir() {
        bail!("{type_name} has no user {user}");
    }
    Ok(Reader::Home(home))
}

pub fn guards_secrets(fleet: &Fleet, text: &str) -> Result<bool> {
    Ok(!secrets_of(fleet, &reader(fleet, text)?)?.is_empty())
}

pub struct Readers<'p> {
    pub prompt: &'p mut Prompt,
    approved: Vec<(PathBuf, Reader)>,
}

impl<'p> Readers<'p> {
    pub fn new(prompt: &'p mut Prompt) -> Self {
        Readers {
            prompt,
            approved: Vec::new(),
        }
    }

    pub fn approve(&mut self, fleet: &Fleet, source: &Path) -> Result<()> {
        if !fs::symlink_metadata(source)?.is_symlink() {
            return Ok(());
        }
        let shared = homes::target(&fleet.dir, source)?;
        let Some(before) = checked::linked_when_written(&fleet.dir, &shared)? else {
            return Ok(());
        };
        let shown = shared.strip_prefix(&fleet.dir).unwrap_or(&shared).display();
        for reader in homes::owners(&fleet.dir, source)? {
            let known = before.contains(&reader)
                || self.approved.contains(&(shared.clone(), reader.clone()));
            if known {
                continue;
            }
            let question = format!(
                "{reader} links {shown}, which was never encrypted for it. Let {reader} open it"
            );
            if !self.prompt.confirm(&question)? {
                bail!("{shown} stays closed to {reader}: remove its link from {reader} first");
            }
            self.approved.push((shared.clone(), reader));
        }
        Ok(())
    }
}

pub fn readers_list(repo: &Repo, source: &Path) -> Result<Vec<String>> {
    Ok(sealed_list(repo, source)?
        .into_iter()
        .map(|recipient| recipient.key)
        .collect())
}

pub fn sealed_list(repo: &Repo, source: &Path) -> Result<Vec<Recipient>> {
    let readers = homes::owners(&repo.dir, source)?;
    Ok(repo.sealed(&readers))
}

fn reseal(
    fleet: &Fleet,
    keys: &Keys,
    machines: &Machines,
    readers: &[Reader],
    approvals: &mut Readers,
) -> Result<()> {
    let mut all: Vec<(PathBuf, Secret)> = Vec::new();
    for reader in readers {
        for secret in secrets_of(fleet, reader)? {
            let file = match fs::symlink_metadata(&secret.path)?.is_symlink() {
                true => homes::target(&fleet.dir, &secret.path)?,
                false => secret.path.clone(),
            };
            if !all.iter().any(|(known, _)| *known == file) {
                all.push((file, secret));
            }
        }
    }
    if all.is_empty() {
        return Ok(());
    }
    println!("Re-encrypting:");
    let pairs: Vec<(String, String)> = all
        .iter()
        .map(|(_, secret)| (secret.shown.clone(), secret.readers.join(", ")))
        .collect();
    style::print_pairs(&pairs);
    let mut sealed = Vec::new();
    for (file, secret) in &all {
        approvals.approve(fleet, &secret.path)?;
        let owners = homes::owners(&fleet.dir, &secret.path)?;
        let recipients = machines.sealed(&owners, Some(&fleet.recovery));
        sealed.push((file, encrypt_for(&recipients, &keys.decrypt(file)?)?));
    }
    for (file, bytes) in sealed {
        let mode = fs::metadata(file)?.permissions().mode() & 0o7777;
        file::write(file, &bytes, mode).with_context(|| format!("writing {}", file.display()))?;
    }
    Ok(())
}

struct Secret {
    shown: String,
    path: PathBuf,
    readers: Vec<String>,
}

fn secret(fleet: &Fleet, shown: String, path: PathBuf) -> Result<Secret> {
    let readers = homes::owners(&fleet.dir, &path)?
        .iter()
        .map(Reader::to_string)
        .collect();
    Ok(Secret {
        shown,
        path,
        readers,
    })
}

fn secrets_of(fleet: &Fleet, reader: &Reader) -> Result<Vec<Secret>> {
    let mut all = Vec::new();
    match reader {
        Reader::Root(type_name) => {
            let type_dir = fleet::type_dir(&fleet.dir, type_name);
            for file in fleet::etc_files(&type_dir)? {
                if file.storage == Storage::Encrypted {
                    let shown = Path::new(file::ETC).join(&file.rel).display().to_string();
                    all.push(secret(fleet, shown, file.source)?);
                }
            }
            for path in services::sealed(&type_dir)? {
                let shown = path.strip_prefix(&fleet.dir)?.display().to_string();
                all.push(secret(fleet, shown, path)?);
            }
        }
        Reader::Home(home) => {
            let source = home.source(&fleet.dir);
            for rel in walk::files(&source)? {
                if let Some(shown) = home_secret(&rel) {
                    all.push(secret(fleet, shown, source.join(&rel))?);
                }
            }
        }
    }
    all.sort_by(|a, b| a.shown.cmp(&b.shown));
    Ok(all)
}

fn home_secret(rel: &Path) -> Option<String> {
    let secret = match rel.starts_with(homes::SECRETS) {
        true => rel.to_string_lossy().ends_with(AGE),
        false => chezmoi::target_of(rel).1,
    };
    secret.then(|| homes::shown(rel))
}

#[cfg(test)]
#[path = "../../tests/trust/mod.rs"]
mod tests;
