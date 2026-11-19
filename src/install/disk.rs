use super::Flags;
use crate::{
    cmd::{self, As},
    prompt::{Choice, Prompt},
    walk,
};
use anyhow::{Context, Result, bail};
use std::{fs, path::Path, str::FromStr};

pub const ROOT: &str = "/mnt";
const MAPPER: &str = "root";
const ESP_TYPE: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";
const ESP_CODE: &str = "ef00";
const XBOOTLDR_CODE: &str = "ea00";
const ROOT_CODE: &str = "8304";
const GIB: u64 = 1 << 30;
const BOOT_SIZE: u64 = GIB;
const ROOT_MIN: u64 = 20 * GIB;
const BOOT_MOUNT: &str = "/run/archiso/bootmnt";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    Wipe,
    Beside,
}

impl FromStr for Layout {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> Result<Layout> {
        match text {
            "wipe" => Ok(Layout::Wipe),
            "beside" => Ok(Layout::Beside),
            _ => bail!("the layout is wipe or beside, not {text}"),
        }
    }
}

pub struct Disk {
    path: String,
    layout: Layout,
}

pub fn choose(flags: &Flags, prompt: &mut Prompt) -> Result<Disk> {
    let disks = list()?;
    let path = match &flags.disk {
        Some(path) if !disks.iter().any(|d| d.name == *path) => {
            bail!("{path} isn't a disk install can use")
        }
        Some(path) => path.clone(),
        None => disks[prompt.choose("Disk", &disks)?].name.clone(),
    };
    let layout = match flags.layout {
        Some(layout) => layout,
        None => {
            let choices = [
                Choice::new("wipe", "erase the whole disk"),
                Choice::new("beside", "use its largest free space, next to Windows"),
            ];
            [Layout::Wipe, Layout::Beside][prompt.choose("Layout", &choices)?]
        }
    };
    Ok(Disk { path, layout })
}

fn list() -> Result<Vec<Choice>> {
    let boot_disk = match cmd::bytes(As::Root, "mountpoint", &["--quiet", BOOT_MOUNT]) {
        Ok(_) => {
            let source = cmd::stdout(
                As::Root,
                "findmnt",
                &["--noheadings", "--output", "SOURCE", BOOT_MOUNT],
            )?;
            let disk = cmd::stdout(
                As::Root,
                "lsblk",
                &[
                    "--noheadings",
                    "--paths",
                    "--output",
                    "PKNAME",
                    source.trim(),
                ],
            )?;
            Some(disk.trim().to_owned())
        }
        Err(_) => None,
    };
    let json = cmd::stdout(
        As::Root,
        "lsblk",
        &[
            "--json",
            "--nodeps",
            "--paths",
            "--output",
            "NAME,SIZE,MODEL,TYPE",
        ],
    )?;
    let json: serde_json::Value = serde_json::from_str(&json)?;
    let text = |device: &serde_json::Value, key: &str| {
        device[key].as_str().unwrap_or_default().trim().to_owned()
    };
    Ok(json["blockdevices"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|device| device["type"] == "disk")
        .filter(|device| {
            let name = text(device, "name");
            let name = name.trim_start_matches("/dev/");
            Path::new("/sys/block").join(name).join("device").exists()
        })
        .filter(|device| boot_disk.as_deref() != Some(&text(device, "name")))
        .map(|device| {
            let about = format!("{}  {}", text(device, "size"), text(device, "model"));
            Choice::new(text(device, "name"), about.trim())
        })
        .collect())
}

pub fn prepare(disk: &Disk, prompt: &mut Prompt) -> Result<()> {
    let name = Path::new(&disk.path)
        .file_name()
        .expect("lsblk names disks by path")
        .to_string_lossy()
        .into_owned();
    match disk.layout {
        Layout::Wipe => println!("This erases everything on {}.", disk.path),
        Layout::Beside => println!(
            "This adds two partitions in the free space of {}.",
            disk.path
        ),
    }
    if prompt.line(&format!("Type {name} to go on:"))? != name {
        bail!("install stopped, and nothing on the disk changed");
    }
    let passphrase = loop {
        let first = prompt.hidden("Disk passphrase:")?;
        if prompt.hidden("Again:")? == first {
            break first;
        }
        println!("They differ.");
    };
    let (root, mounts) = match disk.layout {
        Layout::Wipe => wipe(&disk.path)?,
        Layout::Beside => beside(&disk.path, prompt)?,
    };
    let key = ["--key-file", "-"];
    let format = [
        &["luksFormat", "--type", "luks2", "--batch-mode"],
        &key[..],
        &[&root],
    ]
    .concat();
    cmd::with_input(As::Root, "cryptsetup", &format, passphrase.as_bytes())?;
    let open = [&["open"], &key[..], &[&root, MAPPER]].concat();
    cmd::with_input(As::Root, "cryptsetup", &open, passphrase.as_bytes())?;
    let mapped = format!("/dev/mapper/{MAPPER}");
    cmd::root("mkfs.ext4", &["-q", &mapped])?;
    cmd::root("mount", &[&mapped, ROOT])?;
    for (device, at) in mounts {
        let dir = walk::under(Path::new(ROOT), at);
        fs::create_dir_all(&dir)?;
        cmd::root("mount", &[&device, &dir.to_string_lossy()])?;
    }
    Ok(())
}

pub fn still_open() -> Result<bool> {
    if !Path::new("/dev/mapper").join(MAPPER).exists() {
        return Ok(false);
    }
    if cmd::bytes(As::Root, "mountpoint", &["--quiet", ROOT]).is_err() {
        bail!(
            "an earlier run opened the disk but didn't mount it: close it with `cryptsetup close \
             {MAPPER}`, then run install again"
        );
    }
    Ok(true)
}

pub fn close() -> Result<()> {
    cmd::root("umount", &["--recursive", ROOT])?;
    cmd::root("cryptsetup", &["close", MAPPER])
}

type Partitions = (String, Vec<(String, &'static str)>);

fn wipe(disk: &str) -> Result<Partitions> {
    cmd::root("sgdisk", &["--zap-all", disk])?;
    cmd::root(
        "sgdisk",
        &[
            &format!("--new=1:0:+{}M", BOOT_SIZE >> 20),
            &format!("--typecode=1:{ESP_CODE}"),
            "--new=2:0:0",
            &format!("--typecode=2:{ROOT_CODE}"),
            disk,
        ],
    )?;
    cmd::root("udevadm", &["settle"])?;
    let esp = partition(disk, 1);
    cmd::root("mkfs.fat", &["-F", "32", &esp])?;
    Ok((partition(disk, 2), vec![(esp, "/boot")]))
}

fn beside(disk: &str, prompt: &mut Prompt) -> Result<Partitions> {
    let esps = esps(disk)?;
    if esps.is_empty() {
        bail!("{disk} has no EFI partition to share");
    }
    let esp = esps[prompt.choose("EFI partition to share", &esps)?]
        .name
        .clone();
    let sector: u64 = cmd::stdout(As::Root, "blockdev", &["--getss", disk])?
        .trim()
        .parse()?;
    let bound = |flag| -> Result<u64> {
        Ok(cmd::stdout(As::Root, "sgdisk", &[flag, disk])?
            .lines()
            .last()
            .context("sgdisk printed nothing")?
            .trim()
            .parse()?)
    };
    let (start, end) = (
        bound("--first-aligned-in-largest")?,
        bound("--end-of-largest")?,
    );
    let boot_end = start + BOOT_SIZE / sector - 1;
    if end < boot_end || (end - boot_end) * sector < ROOT_MIN {
        let needed = (BOOT_SIZE + ROOT_MIN) / GIB;
        bail!("{disk}'s largest free space is under {needed} GiB: shrink Windows first");
    }
    let next = numbers(disk)?.into_iter().max().unwrap_or(0) + 1;
    cmd::root(
        "sgdisk",
        &[
            &format!("--new={next}:{start}:{boot_end}"),
            &format!("--typecode={next}:{XBOOTLDR_CODE}"),
            &format!("--new={}:{}:{end}", next + 1, boot_end + 1),
            &format!("--typecode={}:{ROOT_CODE}", next + 1),
            disk,
        ],
    )?;
    cmd::root("udevadm", &["settle"])?;
    let boot = partition(disk, next);
    cmd::root("mkfs.fat", &["-F", "32", &boot])?;
    Ok((
        partition(disk, next + 1),
        vec![(boot, "/boot"), (esp, "/efi")],
    ))
}

fn partitions(disk: &str) -> Result<Vec<serde_json::Value>> {
    let json = cmd::stdout(
        As::Root,
        "lsblk",
        &["--json", "--paths", "--output", "NAME,PARTTYPE,PARTN", disk],
    )?;
    let json: serde_json::Value = serde_json::from_str(&json)?;
    Ok(json["blockdevices"][0]["children"]
        .as_array()
        .cloned()
        .unwrap_or_default())
}

fn esps(disk: &str) -> Result<Vec<Choice>> {
    Ok(partitions(disk)?
        .iter()
        .filter(|p| p["parttype"] == ESP_TYPE)
        .filter_map(|p| p["name"].as_str().map(|name| Choice::new(name, "")))
        .collect())
}

fn numbers(disk: &str) -> Result<Vec<u64>> {
    Ok(partitions(disk)?
        .iter()
        .filter_map(|p| p["partn"].as_u64())
        .collect())
}

fn partition(disk: &str, number: u64) -> String {
    if disk.ends_with(|c: char| c.is_ascii_digit()) {
        format!("{disk}p{number}")
    } else {
        format!("{disk}{number}")
    }
}

#[cfg(test)]
#[path = "../../tests/disk.rs"]
mod tests;
