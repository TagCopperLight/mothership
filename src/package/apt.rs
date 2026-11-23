use super::{Backup, Db, Origin, dpkg::EXTENDED_STATES};
use crate::{
    system::systemd::{self, SYSTEM_DIRS, USER_DIRS},
    util::cmd::{self, As},
    util::file,
};
use anyhow::{Context, Result, anyhow};
use std::{collections::BTreeSet, fs, path::Path};

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

pub fn cannot_install(name: &str) -> Result<Option<String>> {
    let offered = cmd::stdout(As::Root, "apt-cache", &["madison", name])?;
    Ok(offered
        .trim()
        .is_empty()
        .then(|| "It is in none of apt's sources.".to_owned()))
}

pub fn install(names: &[&str]) -> Result<()> {
    apt_get(&["update"])?;
    let restore_config = ["-o", "Dpkg::Options::=--force-confmiss"];
    apt_get(&[&["install", "-y"], &restore_config[..], names].concat())
}

pub fn uninstall(db: &Db, keep: &[&str], names: &[&str]) -> Result<BTreeSet<String>> {
    if !keep.is_empty() {
        cmd::root("apt-mark", &[&["manual"], keep].concat())?;
    }
    let before = autoremovable(&[])?;
    cmd::root("apt-mark", &[&["auto"], names].concat())?;
    super::or_restore(remove_unneeded(db, &before), || {
        cmd::root("apt-mark", &[&["manual"], names].concat())
    })
}

fn remove_unneeded(db: &Db, before: &BTreeSet<String>) -> Result<BTreeSet<String>> {
    let after = autoremovable(&[])?;
    let unneeded: BTreeSet<String> = after.difference(before).cloned().collect();
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
    let bytes = file::if_found(fs::read(&backup.path))
        .with_context(|| format!("reading {}", backup.path.display()))?;
    Ok(bytes.is_some_and(|bytes| crate::util::hash::md5(&bytes) == backup.md5))
}

pub fn keep_only(
    mut keep: BTreeSet<String>,
    confirm: &mut dyn FnMut() -> Result<bool>,
) -> Result<()> {
    let db = Db::load()?;
    keep.extend(db.needing(&runs_the_machine(&db)?));
    let manual = cmd::stdout(As::Root, "apt-mark", &["showmanual"])?;
    let unmark: Vec<&str> = manual.lines().filter(|n| !keep.contains(*n)).collect();
    if unmark.is_empty() {
        return Ok(());
    }
    let states = tempfile::NamedTempFile::new()?;
    if let Some(text) = file::read_if_any(Path::new(EXTENDED_STATES))? {
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
    if !confirm()? {
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
    cmd::status(command, "apt-get", args).map_err(|e| {
        anyhow!(
            "{e:#}\nIf another program holds apt's lock, such as unattended upgrades, apply again \
             once it's done."
        )
    })
}

#[cfg(test)]
#[path = "../../tests/package/apt.rs"]
mod tests;
