use crate::{
    chezmoi, file,
    fleet::{
        self, Fleet, Repo, checked,
        homes::{self, Home},
    },
    github, host,
    prompt::Prompt,
    secrets::{
        AGE, Keys, RECIPIENTS, ROOT_KEY, Recipient, Storage, encrypt_for, format, holds, key_path,
        make_key, public, read, root_key, user_key,
    },
    sign, ssh, style,
    user::User,
    walk,
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

fn owners(fleet: &Fleet) -> Vec<(String, Option<&User>, PathBuf)> {
    let mut owners = vec![("root".to_owned(), None, fleet.type_dir.clone())];
    for member in &fleet.members {
        owners.push((member.name.clone(), Some(member.user()), member.dir.clone()));
    }
    owners
}

pub fn create_keys(fleet: &Fleet) -> Result<()> {
    if sign::machine_key()?.is_none() {
        sign::create_machine_key()?;
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
    let mut keys = Vec::new();
    for (name, user, dir) in owners(fleet) {
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
                trusted: holds(&dir.join(RECIPIENTS), &public)?,
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
                trusted: holds(&dir.join(ssh::KEYS), &public)?,
                public,
                target,
            });
        }
    }
    if let Some(key) = sign::machine_key()? {
        let text = file::read_if_any(&fleet.dir.join(sign::SIGNERS))?.unwrap_or_default();
        keys.push(MachineKey {
            name: SIGNING.to_owned(),
            sort: Sort::Signing,
            trusted: sign::parse_signers(&text)?
                .iter()
                .any(|signer| &signer.key == key.public_key()),
            public: sign::public(&key)?,
            target: sign::SIGNERS.to_owned(),
        });
    }
    Ok(keys)
}

pub fn key(fleet: &Fleet) -> Result<()> {
    create_keys(fleet)?;
    for key in &machine_keys(fleet)? {
        println!("{:7}  {}", key.name, key.public);
    }
    if let Some(command) = trust_command(fleet, &host::name()?)? {
        println!("Run this on a machine that signs for the fleet:");
        println!("  {command}");
    }
    Ok(())
}

pub fn pending(fleet: &Fleet) -> Result<Vec<String>> {
    Ok(machine_keys(fleet)?
        .into_iter()
        .filter(|key| !key.trusted)
        .map(|key| format!("{}={}", key.name, key.public))
        .collect())
}

pub fn trust_command(fleet: &Fleet, hostname: &str) -> Result<Option<String>> {
    let entries = pending(fleet)?;
    let quoted: Vec<String> = entries
        .iter()
        .map(|entry| match entry.contains(' ') {
            true => format!("'{entry}'"),
            false => entry.clone(),
        })
        .collect();
    Ok((!entries.is_empty()).then(|| {
        format!(
            "sudo mothership trust {hostname} {} {}",
            fleet.type_name,
            quoted.join(" ")
        )
    }))
}

pub fn trust_host(
    fleet: &Fleet,
    keys: &Keys,
    host: &str,
    type_name: &str,
    entries: &[String],
    prompt: &mut Prompt,
) -> Result<Vec<github::Key>> {
    let mut readers = Readers::new(prompt);
    let mut uploads = Vec::new();
    for entry in entries {
        let (name, key) = entry
            .split_once('=')
            .with_context(|| format!("{entry} isn't <name>=<key>"))?;
        let text = format!("{type_name}/{name}");
        if name == SIGNING {
            let public = add_signer(fleet, host, type_name, key, readers.prompt)?;
            uploads.push(github::Key {
                title: host.to_owned(),
                public,
                kind: github::Use::Signing,
            });
        } else if key.starts_with("ssh-") {
            let public = add_ssh_key(fleet, &text, host, key)?;
            uploads.push(github::Key {
                title: github::title(host, name),
                public,
                kind: github::Use::Login,
            });
        } else if !holds(&target(fleet, &text)?.list, key)? {
            trust_with(fleet, keys, &text, host, key, &mut readers)?;
        }
    }
    Ok(uploads)
}

fn listed_as(list: &[Recipient], path: &Path, host: &str, key: &str) -> Result<bool> {
    let same = |known: &Recipient| {
        known
            .key
            .split_whitespace()
            .take(2)
            .eq(key.split_whitespace())
    };
    match list.iter().find(|entry| entry.name == host) {
        Some(known) if same(known) => Ok(true),
        Some(_) => bail!(
            "{} already lists another key for {host}: retire it first",
            path.display()
        ),
        None => Ok(false),
    }
}

fn add_ssh_key(fleet: &Fleet, text: &str, host: &str, key: &str) -> Result<String> {
    let Scope::User(home) = target(fleet, text)?.scope else {
        bail!("root has no SSH key in the fleet, only its users do");
    };
    let public = ssh::normalize(key)?;
    let path = home.dir(&fleet.dir).join(ssh::KEYS);
    let mut list = read(&path)?;
    if !listed_as(&list, &path, host, &public)? {
        list.push(Recipient {
            name: host.to_owned(),
            key: public.clone(),
        });
        file::write(&path, format(&list).as_bytes(), 0o644)?;
        println!("{text} on {host} has an SSH key in the fleet now.");
    }
    Ok(public)
}

fn add_signer(
    fleet: &Fleet,
    host: &str,
    type_name: &str,
    key: &str,
    prompt: &mut Prompt,
) -> Result<String> {
    let path = fleet.dir.join(sign::SIGNERS);
    let mut list = read(&path)?;
    let public = ssh::normalize(key)?;
    if listed_as(&list, &path, host, &public)? {
        return Ok(public);
    }
    let own = format!("types/{type_name}");
    let question = format!(
        "{host} may change: Enter for {own}, * for everything, or folders separated by spaces ›"
    );
    let line = loop {
        let answer = prompt.line_or_empty(&question)?;
        let words: Vec<&str> = match answer.as_str() {
            "" => vec![own.as_str()],
            "*" => Vec::new(),
            folders => folders.split_whitespace().collect(),
        };
        match reach(fleet, &words) {
            Ok(()) => break [&[public.as_str()][..], &words].concat().join(" "),
            Err(e) => println!("  {e:#}"),
        }
    };
    list.push(Recipient {
        name: host.to_owned(),
        key: line,
    });
    file::write(&path, format(&list).as_bytes(), 0o644)?;
    println!("{host} signs for the fleet now.");
    Ok(public)
}

fn reach(fleet: &Fleet, folders: &[&str]) -> Result<()> {
    if let sign::Reach::Folders(folders) = sign::Reach::parse(folders)? {
        for folder in folders {
            if !fleet.dir.join(&folder).is_dir() {
                bail!("the fleet has no folder {}", folder.display());
            }
        }
    }
    Ok(())
}

fn every_list(fleet: &Fleet) -> Result<Vec<Target>> {
    let mut all = Vec::new();
    for type_name in fleet::types(&fleet.dir)? {
        all.push(target(fleet, &format!("{type_name}/root"))?);
        let users = fleet::type_dir(&fleet.dir, &type_name).join(fleet::USERS);
        for dir in walk::children(&users)? {
            let user = dir.file_name().unwrap_or_default().to_string_lossy();
            all.push(target(fleet, &format!("{type_name}/{user}"))?);
        }
    }
    Ok(all)
}

pub fn retire_host(fleet: &Fleet, host: &str, prompt: &mut Prompt) -> Result<()> {
    let mut readers = Readers::new(prompt);
    let mut found = false;
    for target in every_list(fleet)? {
        if read(&target.list)?.iter().any(|r| r.name == host) {
            retire(fleet, &target.text, host, &mut readers)?;
            found = true;
        }
        if let Scope::User(home) = &target.scope
            && drop_named(&home.dir(&fleet.dir).join(ssh::KEYS), host)?
        {
            println!("{} on {host} has no SSH key in the fleet now.", target.text);
            found = true;
        }
    }
    if drop_named(&fleet.dir.join(sign::SIGNERS), host)? {
        println!("{host} signs nothing now.");
        found = true;
    }
    if !found {
        bail!("the fleet holds no key named {host}");
    }
    Ok(())
}

pub fn reseal_everything(
    fleet: &Fleet,
    keys: &Keys,
    dropped: &str,
    prompt: &mut Prompt,
) -> Result<()> {
    let mut readers = Readers::new(prompt);
    for target in every_list(fleet)? {
        let before = read(&target.list)?;
        let mut list = before.clone();
        list.retain(|r| r.key != dropped);
        if list == before && secrets_of(fleet, &target)?.is_empty() {
            continue;
        }
        rewrite(fleet, keys, &target, &list, &mut readers)?;
    }
    Ok(())
}

fn drop_named(path: &Path, name: &str) -> Result<bool> {
    let mut list = read(path)?;
    let before = list.len();
    list.retain(|entry| entry.name != name);
    if list.len() == before {
        return Ok(false);
    }
    file::write(path, format(&list).as_bytes(), 0o644)?;
    Ok(true)
}

enum Scope {
    Root,
    User(Home),
}

struct Target {
    text: String,
    type_name: String,
    scope: Scope,
    list: PathBuf,
}

fn target(fleet: &Fleet, text: &str) -> Result<Target> {
    let (type_name, scope) = text
        .split_once('/')
        .with_context(|| format!("{text} is not <type>/root or <type>/<user>"))?;
    let type_dir = fleet::type_dir(&fleet.dir, type_name);
    if type_name.is_empty() || type_name.contains("..") || !type_dir.is_dir() {
        bail!("the fleet has no type {type_name}");
    }
    let (scope, list) = match scope {
        "root" => (Scope::Root, type_dir.join(RECIPIENTS)),
        user => {
            let home = Home {
                type_name: type_name.to_owned(),
                user: user.to_owned(),
            };
            let dir = home.dir(&fleet.dir);
            if user.is_empty() || user.contains('/') || user.contains("..") || !dir.is_dir() {
                bail!("{type_name} has no user {user}");
            }
            (Scope::User(home), dir.join(RECIPIENTS))
        }
    };
    Ok(Target {
        text: text.to_owned(),
        type_name: type_name.to_owned(),
        scope,
        list,
    })
}

pub fn guards_secrets(fleet: &Fleet, text: &str) -> Result<bool> {
    Ok(!secrets_of(fleet, &target(fleet, text)?)?.is_empty())
}

pub fn trust_with(
    fleet: &Fleet,
    keys: &Keys,
    text: &str,
    name: &str,
    key: &str,
    readers: &mut Readers,
) -> Result<()> {
    let target = target(fleet, text)?;
    key.parse::<x25519::Recipient>()
        .map_err(|e| anyhow!("{key}: {e}"))?;
    if name.is_empty() || name.contains(['\n', '#']) {
        bail!("a key's name can't be empty or hold a newline or #");
    }
    let mut list = read(&target.list)?;
    if let Some(known) = list.iter().find(|r| r.key == key) {
        bail!("{text} already holds this key, as {}", known.name);
    }
    if list.iter().any(|r| r.name == name) {
        bail!("{text} already holds a key named {name}: run `sudo mothership retire {name}` first");
    }
    list.push(Recipient {
        name: name.to_owned(),
        key: key.to_owned(),
    });
    let rewritten = rewrite(fleet, keys, &target, &list, readers)?;
    println!(
        "{name} opens the {} secrets of {text} now.",
        rewritten.len()
    );
    Ok(())
}

pub fn retire(fleet: &Fleet, text: &str, name: &str, readers: &mut Readers) -> Result<()> {
    let target = target(fleet, text)?;
    let mut list = read(&target.list)?;
    let retired = list
        .iter()
        .find(|r| r.name == name)
        .with_context(|| format!("{text} holds no {name}"))?
        .key
        .clone();
    list.retain(|r| r.name != name);
    let rewritten = rewrite(fleet, fleet.keys(), &target, &list, readers)?;
    println!("{name} is gone from {text}.");
    if rewritten.is_empty() {
        return Ok(());
    }
    println!(
        "git history keeps the versions {name} opens, and that machine may keep copies, so change these:"
    );
    for secret in &rewritten {
        println!("  {}", secret.shown);
    }
    let still: Vec<&Rewritten> = rewritten
        .iter()
        .filter(|secret| secret.recipients.iter().any(|r| r.key == retired))
        .collect();
    if !still.is_empty() {
        println!("{name}'s key still opens these, through another home that shares them:");
        let others = |secret: &Rewritten| {
            let others: Vec<&str> = secret
                .readers
                .iter()
                .map(String::as_str)
                .filter(|reader| *reader != text)
                .collect();
            others.join(", ")
        };
        let pairs: Vec<(String, String)> = still
            .iter()
            .map(|secret| (secret.shown.clone(), others(secret)))
            .collect();
        style::print_pairs(&pairs);
    }
    Ok(())
}

pub struct Readers<'p> {
    pub prompt: &'p mut Prompt,
    approved: Vec<(PathBuf, Home)>,
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
        for home in homes::owners(&fleet.dir, source)? {
            let known = before.contains(&home.dir(Path::new("")))
                || self.approved.contains(&(shared.clone(), home.clone()));
            if known {
                continue;
            }
            let question = format!(
                "{home} links {shown}, which was never encrypted for it. Let {home} open it"
            );
            if !self.prompt.confirm(&question)? {
                bail!("{shown} stays closed to {home}: remove its link from {home} first");
            }
            self.approved.push((shared.clone(), home));
        }
        Ok(())
    }
}

pub fn home_list(repo: &Repo, source: &Path) -> Result<String> {
    Ok(format(
        &repo.sealed_for(home_recipients(&repo.dir, source, None)?),
    ))
}

fn home_recipients(
    fleet_dir: &Path,
    source: &Path,
    replacing: Option<(&Path, &[Recipient])>,
) -> Result<Vec<Recipient>> {
    let mut all: Vec<Recipient> = Vec::new();
    for home in homes::owners(fleet_dir, source)? {
        let path = home.dir(fleet_dir).join(RECIPIENTS);
        let list = match replacing {
            Some((replaced, list)) if replaced == path => list.to_vec(),
            _ => read(&path)?,
        };
        for recipient in list {
            if !all.iter().any(|known| known.key == recipient.key) {
                all.push(recipient);
            }
        }
    }
    Ok(all)
}

struct Rewritten {
    shown: String,
    readers: Vec<String>,
    recipients: Vec<Recipient>,
}

fn rewrite(
    fleet: &Fleet,
    keys: &Keys,
    target: &Target,
    list: &[Recipient],
    readers: &mut Readers,
) -> Result<Vec<Rewritten>> {
    let all = secrets_of(fleet, target)?;
    if !all.is_empty() {
        println!("Re-encrypting for {}:", target.text);
        let pairs: Vec<(String, String)> = all
            .iter()
            .map(|secret| (secret.shown.clone(), secret.readers.join(", ")))
            .collect();
        style::print_pairs(&pairs);
    }
    let mut sealed = Vec::new();
    for secret in all {
        if let Source::Home(path) = &secret.source {
            readers.approve(fleet, path)?;
        }
        let recipients = fleet.sealed_for(match &secret.source {
            Source::Home(path) => home_recipients(&fleet.dir, path, Some((&target.list, list)))?,
            Source::Etc(_) => list.to_vec(),
        });
        let file = secret.source.file(&fleet.dir)?;
        let bytes = encrypt_for(&recipients, &keys.decrypt(&file)?)?;
        sealed.push((file, bytes, secret, recipients));
    }
    let mut rewritten = Vec::new();
    for (file, bytes, secret, recipients) in sealed {
        let mode = fs::metadata(&file)?.permissions().mode() & 0o7777;
        file::write(&file, &bytes, mode).with_context(|| format!("writing {}", file.display()))?;
        rewritten.push(Rewritten {
            shown: secret.shown,
            readers: secret.readers,
            recipients,
        });
    }
    file::write(&target.list, format(list).as_bytes(), 0o644)
        .with_context(|| format!("writing {}", target.list.display()))?;
    Ok(rewritten)
}

enum Source {
    Home(PathBuf),
    Etc(PathBuf),
}

impl Source {
    fn file(&self, fleet_dir: &Path) -> Result<PathBuf> {
        match self {
            Source::Home(path) if fs::symlink_metadata(path)?.is_symlink() => {
                homes::target(fleet_dir, path)
            }
            Source::Home(path) | Source::Etc(path) => Ok(path.clone()),
        }
    }
}

struct Secret {
    shown: String,
    source: Source,
    readers: Vec<String>,
}

fn secrets_of(fleet: &Fleet, target: &Target) -> Result<Vec<Secret>> {
    let mut all = Vec::new();
    match &target.scope {
        Scope::Root => {
            let type_dir = fleet::type_dir(&fleet.dir, &target.type_name);
            for file in fleet::etc_files(&type_dir)? {
                if file.storage == Storage::Encrypted {
                    all.push(Secret {
                        shown: Path::new(file::ETC).join(&file.rel).display().to_string(),
                        source: Source::Etc(file.source),
                        readers: vec![target.text.clone()],
                    });
                }
            }
        }
        Scope::User(home) => {
            let source = home.source(&fleet.dir);
            for rel in walk::files(&source)? {
                if let Some(shown) = home_secret(&rel) {
                    let path = source.join(&rel);
                    let readers = homes::owners(&fleet.dir, &path)?
                        .iter()
                        .map(Home::to_string)
                        .collect();
                    all.push(Secret {
                        shown,
                        source: Source::Home(path),
                        readers,
                    });
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
#[path = "../tests/trust.rs"]
mod tests;
