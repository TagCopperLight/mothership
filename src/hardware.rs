use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer, de};
use std::{collections::BTreeSet, fs, io::ErrorKind, path::Path};

/// What mothership can find on a machine. Each device type says what each one brings.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Hardware {
    CpuIntel,
    CpuAmd,
    GpuIntel,
    GpuAmd,
    GpuNvidia,
    Battery,
    /// The fingerprint sensor of the MateBook 14.
    Gxfp5130,
}

impl Hardware {
    const ALL: [Hardware; 7] = [
        Hardware::CpuIntel,
        Hardware::CpuAmd,
        Hardware::GpuIntel,
        Hardware::GpuAmd,
        Hardware::GpuNvidia,
        Hardware::Battery,
        Hardware::Gxfp5130,
    ];

    /// The key under `[hardware]` in system.toml.
    pub fn name(self) -> &'static str {
        match self {
            Hardware::CpuIntel => "cpu-intel",
            Hardware::CpuAmd => "cpu-amd",
            Hardware::GpuIntel => "gpu-intel",
            Hardware::GpuAmd => "gpu-amd",
            Hardware::GpuNvidia => "gpu-nvidia",
            Hardware::Battery => "battery",
            Hardware::Gxfp5130 => "gxfp5130",
        }
    }
}

impl<'de> Deserialize<'de> for Hardware {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Hardware::ALL
            .into_iter()
            .find(|piece| piece.name() == name)
            .ok_or_else(|| de::Error::custom(format!("mothership can't detect {name}")))
    }
}

/// `root` is `/` on a real machine.
pub fn detect(root: &Path) -> Result<BTreeSet<Hardware>> {
    let mut found = BTreeSet::from([cpu(root)?]);
    found.extend(gpus(root)?);
    if battery(root)? {
        found.insert(Hardware::Battery);
    }
    if entries(&root.join("sys/bus/acpi/devices"))?
        .iter()
        .any(|name| name.starts_with("GXFP5130:"))
    {
        found.insert(Hardware::Gxfp5130);
    }
    Ok(found)
}

fn cpu(root: &Path) -> Result<Hardware> {
    let path = root.join("proc/cpuinfo");
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let vendor = text
        .lines()
        .find_map(|line| line.strip_prefix("vendor_id"))
        .and_then(|rest| rest.split(':').nth(1))
        .map(str::trim)
        .context("/proc/cpuinfo names no CPU vendor")?;
    Ok(match vendor {
        "GenuineIntel" => Hardware::CpuIntel,
        "AuthenticAMD" => Hardware::CpuAmd,
        other => bail!("unknown CPU vendor {other}"),
    })
}

/// Display controllers are PCI class 0x03. Virtual ones, from other vendors, need no driver.
fn gpus(root: &Path) -> Result<BTreeSet<Hardware>> {
    let dir = root.join("sys/bus/pci/devices");
    let mut found = BTreeSet::new();
    for name in entries(&dir)? {
        let device = dir.join(name);
        if !read(&device.join("class"))?.starts_with("0x03") {
            continue;
        }
        found.extend(match read(&device.join("vendor"))?.as_str() {
            "0x8086" => Some(Hardware::GpuIntel),
            "0x1002" => Some(Hardware::GpuAmd),
            "0x10de" => Some(Hardware::GpuNvidia),
            _ => None,
        });
    }
    Ok(found)
}

/// A wireless mouse or headset shows up as a battery too, with the scope `Device`.
fn battery(root: &Path) -> Result<bool> {
    let dir = root.join("sys/class/power_supply");
    for name in entries(&dir)? {
        let supply = dir.join(name);
        if read(&supply.join("type"))? != "Battery" {
            continue;
        }
        match fs::read_to_string(supply.join("scope")) {
            Ok(scope) if scope.trim() == "Device" => continue,
            Err(e) if e.kind() != ErrorKind::NotFound => return Err(e.into()),
            _ => return Ok(true),
        }
    }
    Ok(false)
}

/// A machine without the bus has no folder for it.
fn entries(dir: &Path) -> Result<Vec<String>> {
    match fs::read_dir(dir) {
        Ok(entries) => entries
            .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
            .collect(),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", dir.display())),
    }
}

fn read(path: &Path) -> Result<String> {
    Ok(fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?
        .trim()
        .to_owned())
}

#[cfg(test)]
#[path = "../tests/hardware.rs"]
mod tests;
