use super::Flags;
use crate::{
    cmd::{self, As},
    prompt::Prompt,
};
use anyhow::{Context, Result, bail};
use std::{fs, path::Path, str::FromStr};

/// Where the ISO mounts the new system.
pub const ROOT: &str = "/mnt";
/// gpt-auto opens the root partition under this name, so the ISO uses it too.
const MAPPER: &str = "root";
const ESP_TYPE: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";
const BOOT_SIZE: u64 = 1 << 30;
const ROOT_MIN: u64 = 20 << 30;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    /// The whole disk: a boot partition, then the encrypted root.
    Wipe,
    /// The largest free space, next to Windows, whose EFI partition systemd-boot shares.
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
        Some(path) if !disks.iter().any(|(p, _)| p == path) => {
            bail!("{path} isn't a disk install can use")
        }
        Some(path) => path.clone(),
        None if disks.is_empty() => bail!("the machine has no disk to install on"),
        None => {
            let shown: Vec<String> = disks.iter().map(|(p, d)| format!("{p}  {d}")).collect();
            disks[prompt.choose("Disk", &shown)?].0.clone()
        }
    };
    let layout = match flags.layout {
        Some(layout) => layout,
        None => {
            let choices = [
                "wipe    erase the whole disk".to_owned(),
                "beside  use its largest free space, next to Windows".to_owned(),
            ];
            [Layout::Wipe, Layout::Beside][prompt.choose("Layout", &choices)?]
        }
    };
    Ok(Disk { path, layout })
}

/// The machine's real disks, leaving out the one the ISO started from.
fn list() -> Result<Vec<(String, String)>> {
    let boot = cmd::stdout(
        As::Root,
        "findmnt",
        &["--noheadings", "--output", "SOURCE", "/run/archiso/bootmnt"],
    )
    .ok()
    .map(|source| source.trim().to_owned());
    let boot_disk = match boot {
        Some(source) => Some(cmd::stdout(
            As::Root,
            "lsblk",
            &["--noheadings", "--paths", "--output", "PKNAME", &source],
        )?),
        None => None,
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
        // zram also shows as a disk, with no device behind it.
        .filter(|device| {
            let name = text(device, "name");
            let name = name.trim_start_matches("/dev/");
            Path::new("/sys/block").join(name).join("device").exists()
        })
        .filter(|device| boot_disk.as_deref().map(str::trim) != Some(&text(device, "name")))
        .map(|device| {
            let about = format!("{}  {}", text(device, "size"), text(device, "model"));
            (text(device, "name"), about.trim().to_owned())
        })
        .collect())
}

/// Partitions, encrypts and mounts the disk under ROOT, once I type its name.
pub fn prepare(disk: &Disk, prompt: &mut Prompt) -> Result<()> {
    let name = Path::new(&disk.path)
        .file_name()
        .context("a disk path has a name")?
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
        let dir = Path::new(ROOT).join(at.trim_start_matches('/'));
        fs::create_dir_all(&dir)?;
        cmd::root("mount", &[&device, &dir.to_string_lossy()])?;
    }
    Ok(())
}

/// Whether an earlier run opened the disk and mounted it, which only the disk step does.
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

/// gpt-auto finds both partitions by their type, so no file names them.
fn wipe(disk: &str) -> Result<Partitions> {
    cmd::root("sgdisk", &["--zap-all", disk])?;
    cmd::root(
        "sgdisk",
        &[
            "--new=1:0:+1G",
            "--typecode=1:ef00",
            "--new=2:0:0",
            "--typecode=2:8304",
            disk,
        ],
    )?;
    cmd::root("udevadm", &["settle"])?;
    let esp = partition(disk, 1);
    cmd::root("mkfs.fat", &["-F", "32", &esp])?;
    Ok((partition(disk, 2), vec![(esp, "/boot")]))
}

/// systemd-boot goes in Windows' EFI partition, which is often too small for the kernel, so
/// the kernel goes in its own boot partition (XBOOTLDR).
fn beside(disk: &str, prompt: &mut Prompt) -> Result<Partitions> {
    let esps = esps(disk)?;
    let esp = match esps.len() {
        0 => bail!("{disk} has no EFI partition to share"),
        _ => esps[prompt.choose("EFI partition to share", &esps)?].clone(),
    };
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
        bail!("{disk}'s largest free space is under 21 GiB: shrink Windows first");
    }
    let next = numbers(disk)?.into_iter().max().unwrap_or(0) + 1;
    cmd::root(
        "sgdisk",
        &[
            &format!("--new={next}:{start}:{boot_end}"),
            &format!("--typecode={next}:ea00"),
            &format!("--new={}:{}:{end}", next + 1, boot_end + 1),
            &format!("--typecode={}:8304", next + 1),
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

fn esps(disk: &str) -> Result<Vec<String>> {
    Ok(partitions(disk)?
        .iter()
        .filter(|p| p["parttype"] == ESP_TYPE)
        .filter_map(|p| p["name"].as_str().map(str::to_owned))
        .collect())
}

fn numbers(disk: &str) -> Result<Vec<u64>> {
    Ok(partitions(disk)?
        .iter()
        .filter_map(|p| p["partn"].as_u64())
        .collect())
}

/// The kernel adds a `p` between a name that ends in a digit and the number: nvme0n1p2.
fn partition(disk: &str, number: u64) -> String {
    if disk.ends_with(|c: char| c.is_ascii_digit()) {
        format!("{disk}p{number}")
    } else {
        format!("{disk}{number}")
    }
}
