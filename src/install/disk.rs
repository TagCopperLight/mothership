use super::answers::Flags;
use crate::{
    cmd::{self, As},
    prompt::{Choice, Prompt},
    walk,
};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{fs, path::Path, str::FromStr};

pub const ROOT: &str = "/mnt";
pub const MAPPER: &str = "root";
const GIB: u64 = 1 << 30;
const LABELS: [&str; 2] = ["mothership-boot", "mothership-root"];
const BOOT_SIZE: u64 = GIB;
const ROOT_MIN: u64 = 20 * GIB;
const BOOT_MOUNT: &str = "/run/archiso/bootmnt";
const COLUMNS: &str =
    "NAME,TYPE,SIZE,MODEL,RO,UUID,LABEL,FSTYPE,PARTLABEL,PARTTYPE,PARTN,MOUNTPOINTS";

struct PartType {
    code: &'static str,
    guid: &'static str,
}

const ESP: PartType = PartType {
    code: "ef00",
    guid: "c12a7328-f81f-11d2-ba4b-00a0c93ec93b",
};
const XBOOTLDR: PartType = PartType {
    code: "ea00",
    guid: "bc13c2ff-59e6-4262-a352-b275fd6f7172",
};
const LINUX_ROOT: PartType = PartType {
    code: "8304",
    guid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709",
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    Wipe,
    Beside,
}

impl Layout {
    const ALL: [Layout; 2] = [Layout::Wipe, Layout::Beside];

    fn name(self) -> &'static str {
        match self {
            Layout::Wipe => "wipe",
            Layout::Beside => "beside",
        }
    }

    fn about(self) -> &'static str {
        match self {
            Layout::Wipe => "erase the whole disk",
            Layout::Beside => "use its largest free space, next to Windows",
        }
    }
}

impl FromStr for Layout {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> Result<Layout> {
        Layout::ALL
            .into_iter()
            .find(|layout| layout.name() == text)
            .with_context(|| format!("the layout is wipe or beside, not {text}"))
    }
}

#[derive(Deserialize)]
struct Lsblk {
    blockdevices: Vec<Device>,
}

#[derive(Deserialize)]
struct Device {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    size: String,
    model: Option<String>,
    ro: bool,
    uuid: Option<String>,
    label: Option<String>,
    fstype: Option<String>,
    partlabel: Option<String>,
    parttype: Option<String>,
    partn: Option<u64>,
    #[serde(default)]
    mountpoints: Vec<Option<String>>,
    #[serde(default)]
    children: Vec<Device>,
}

impl Device {
    fn tree(&self) -> Vec<&Device> {
        let mut all = vec![self];
        for child in &self.children {
            all.extend(child.tree());
        }
        all
    }

    fn about(&self) -> String {
        format!(
            "{}  {}",
            self.size,
            self.model.as_deref().unwrap_or_default().trim()
        )
        .trim()
        .to_owned()
    }

    fn mounted_at(&self) -> Option<&str> {
        self.mountpoints.iter().flatten().next().map(String::as_str)
    }

    fn has_type(&self, kind: &PartType) -> bool {
        self.parttype.as_deref() == Some(kind.guid)
    }

    fn busy(&self) -> Option<String> {
        for device in self.tree() {
            if let Some(at) = device.mounted_at() {
                return Some(format!("{} is in use as {at}", device.name));
            }
        }
        self.children
            .iter()
            .filter(|child| child.kind != "part")
            .chain(self.children.iter().flat_map(|part| &part.children))
            .next()
            .map(|holder| format!("{} is open as {}", holder.kind, holder.name))
    }
}

fn lsblk(args: &[&str]) -> Result<Vec<Device>> {
    let args = [&["--json", "--paths", "--output", COLUMNS], args].concat();
    let json = cmd::stdout(As::Root, "lsblk", &args)?;
    Ok(serde_json::from_str::<Lsblk>(&json)
        .context("reading lsblk's output")?
        .blockdevices)
}

#[derive(Default)]
struct Iso {
    paths: Vec<String>,
    labels: Vec<String>,
    uuids: Vec<String>,
}

impl Iso {
    fn find() -> Result<Iso> {
        let mut iso = Iso::default();
        let cmdline = fs::read_to_string("/proc/cmdline").context("reading /proc/cmdline")?;
        for word in cmdline.split_whitespace() {
            match word.split_once('=') {
                Some(("archisodevice", path)) => iso.paths.push(canonical(path)?),
                Some(("archisolabel", label)) => iso.labels.push(label.to_owned()),
                Some(("archisosearchuuid", uuid)) => iso.uuids.push(uuid.to_owned()),
                _ => {}
            }
        }
        let mountinfo =
            fs::read_to_string("/proc/self/mountinfo").context("reading /proc/self/mountinfo")?;
        for line in mountinfo.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let after = fields.iter().position(|f| *f == "-");
            if let (Some(&BOOT_MOUNT), Some(after)) = (fields.get(4), after)
                && let Some(source) = fields.get(after + 2)
            {
                iso.paths.push(canonical(source)?);
            }
        }
        Ok(iso)
    }

    fn holds(&self, disk: &Device) -> bool {
        disk.tree().iter().any(|d| {
            self.paths.contains(&d.name)
                || d.label.as_ref().is_some_and(|l| self.labels.contains(l))
                || d.uuid.as_ref().is_some_and(|u| self.uuids.contains(u))
        })
    }
}

fn canonical(path: &str) -> Result<String> {
    let found =
        walk::if_found(fs::canonicalize(path)).with_context(|| format!("finding {path}"))?;
    Ok(match found {
        Some(found) => found.to_string_lossy().into_owned(),
        None => path.to_owned(),
    })
}

fn disks() -> Result<Vec<Device>> {
    let iso = Iso::find()?;
    Ok(lsblk(&[])?
        .into_iter()
        .filter(|device| device.kind == "disk" && !device.ro)
        .filter(|device| {
            let name = device.name.trim_start_matches("/dev/");
            Path::new("/sys/block").join(name).join("device").exists()
        })
        .filter(|device| !iso.holds(device))
        .collect())
}

pub struct Disk {
    device: Device,
    layout: Layout,
}

pub fn choose(flags: &Flags, prompt: &mut Prompt) -> Result<Disk> {
    let mut disks = disks()?;
    let index = match &flags.disk {
        Some(path) => disks
            .iter()
            .position(|d| d.name == *path)
            .with_context(|| format!("{path} isn't a disk install can use"))?,
        None => {
            let choices: Vec<Choice> = disks
                .iter()
                .map(|d| Choice::new(&d.name, d.about()))
                .collect();
            prompt.choose("Disk", &choices)?
        }
    };
    let device = disks.swap_remove(index);
    if let Some(why) = device.busy() {
        bail!("{} is in use: {why}", device.name);
    }
    let layout = match flags.layout {
        Some(layout) => layout,
        None => {
            let choices: Vec<Choice> = Layout::ALL
                .iter()
                .map(|l| Choice::new(l.name(), l.about()))
                .collect();
            Layout::ALL[prompt.choose("Layout", &choices)?]
        }
    };
    Ok(Disk { device, layout })
}

enum Plan {
    Wipe,
    Beside { esp: String, space: Space },
}

enum Space {
    Free {
        next: u64,
        start: u64,
        boot_end: u64,
        end: u64,
    },
    Earlier {
        boot: String,
        root: String,
    },
}

pub fn prepare(disk: &Disk, prompt: &mut Prompt) -> Result<()> {
    let path = &disk.device.name;
    println!("{path}  {}", disk.device.about());
    for part in &disk.device.children {
        let name = part.label.as_deref().or(part.partlabel.as_deref());
        println!(
            "  {}  {}  {}  {}",
            part.name,
            part.size,
            part.fstype.as_deref().unwrap_or_default(),
            name.unwrap_or_default()
        );
    }
    let plan = match disk.layout {
        Layout::Wipe => Plan::Wipe,
        Layout::Beside => plan_beside(&disk.device, prompt)?,
    };
    match &plan {
        Plan::Wipe => println!("This erases everything on {path}."),
        Plan::Beside {
            space: Space::Free { .. },
            ..
        } => println!("This adds two partitions in the free space of {path}."),
        Plan::Beside {
            space: Space::Earlier { boot, root },
            ..
        } => println!("This erases {boot} and {root}, which an earlier install made."),
    }
    let name = Path::new(path)
        .file_name()
        .with_context(|| format!("{path} names no device"))?
        .to_string_lossy()
        .into_owned();
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
    let (root, mounts) = match plan {
        Plan::Wipe => wipe(path)?,
        Plan::Beside { esp, space } => beside(path, esp, space)?,
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
            &format!("--typecode=1:{}", ESP.code),
            "--new=2:0:0",
            &format!("--typecode=2:{}", LINUX_ROOT.code),
            disk,
        ],
    )?;
    cmd::root("udevadm", &["settle"])?;
    let esp = partition(disk, 1);
    cmd::root("mkfs.fat", &["-F", "32", &esp])?;
    Ok((partition(disk, 2), vec![(esp, "/boot")]))
}

fn plan_beside(disk: &Device, prompt: &mut Prompt) -> Result<Plan> {
    let path = &disk.name;
    let esps: Vec<Choice> = disk
        .children
        .iter()
        .filter(|p| p.has_type(&ESP))
        .map(|p| Choice::new(&p.name, &p.size))
        .collect();
    if esps.is_empty() {
        bail!("{path} has no EFI partition to share");
    }
    let esp = esps[prompt.choose("EFI partition to share", &esps)?]
        .name
        .clone();
    let labelled = |label: &str| {
        disk.children
            .iter()
            .find(|p| p.partlabel.as_deref() == Some(label))
    };
    let space = match (labelled(LABELS[0]), labelled(LABELS[1])) {
        (None, None) => free_space(disk)?,
        (Some(boot), Some(root))
            if boot.has_type(&XBOOTLDR)
                && root.has_type(&LINUX_ROOT)
                && root.partn == boot.partn.map(|n| n + 1) =>
        {
            Space::Earlier {
                boot: boot.name.clone(),
                root: root.name.clone(),
            }
        }
        (boot, root) => {
            let names: Vec<&str> = [boot, root]
                .into_iter()
                .flatten()
                .map(|p| p.name.as_str())
                .collect();
            bail!(
                "{} come from an earlier install, but not as the pair install makes: delete them, \
                 then run install again",
                names.join(" and ")
            );
        }
    };
    Ok(Plan::Beside { esp, space })
}

fn free_space(disk: &Device) -> Result<Space> {
    let path = &disk.name;
    let sector: u64 = cmd::stdout(As::Root, "blockdev", &["--getss", path])?
        .trim()
        .parse()?;
    let bound = |flag| -> Result<u64> {
        Ok(cmd::stdout(As::Root, "sgdisk", &[flag, path])?
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
        bail!("{path}'s largest free space is under {needed} GiB: shrink Windows first");
    }
    let next = disk
        .children
        .iter()
        .filter_map(|p| p.partn)
        .max()
        .unwrap_or(0)
        + 1;
    Ok(Space::Free {
        next,
        start,
        boot_end,
        end,
    })
}

fn beside(disk: &str, esp: String, space: Space) -> Result<Partitions> {
    let (boot, root) = match space {
        Space::Earlier { boot, root } => (boot, root),
        Space::Free {
            next,
            start,
            boot_end,
            end,
        } => {
            cmd::root(
                "sgdisk",
                &[
                    &format!("--new={next}:{start}:{boot_end}"),
                    &format!("--typecode={next}:{}", XBOOTLDR.code),
                    &format!("--change-name={next}:{}", LABELS[0]),
                    &format!("--new={}:{}:{end}", next + 1, boot_end + 1),
                    &format!("--typecode={}:{}", next + 1, LINUX_ROOT.code),
                    &format!("--change-name={}:{}", next + 1, LABELS[1]),
                    disk,
                ],
            )?;
            cmd::root("udevadm", &["settle"])?;
            (partition(disk, next), partition(disk, next + 1))
        }
    };
    cmd::root("mkfs.fat", &["-F", "32", &boot])?;
    Ok((root, vec![(boot, "/boot"), (esp, "/efi")]))
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
