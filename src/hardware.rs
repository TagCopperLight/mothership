use crate::walk;
use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, de};
use std::{collections::BTreeSet, fs, io::ErrorKind, path::Path};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Hardware {
    CpuIntel,
    CpuAmd,
    GpuIntel,
    GpuAmd,
    GpuNvidia,
    Battery,
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

pub fn detect(root: &Path) -> Result<BTreeSet<Hardware>> {
    let mut found: BTreeSet<Hardware> = cpu(root)?.into_iter().collect();
    found.extend(gpus(root)?);
    if battery(root)? {
        found.insert(Hardware::Battery);
    }
    if walk::children(&root.join("sys/bus/acpi/devices"))?
        .iter()
        .filter_map(|path| path.file_name())
        .any(|name| name.to_string_lossy().starts_with("GXFP5130:"))
    {
        found.insert(Hardware::Gxfp5130);
    }
    Ok(found)
}

fn cpu(root: &Path) -> Result<Option<Hardware>> {
    let path = root.join("proc/cpuinfo");
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let vendor = text
        .lines()
        .find_map(|line| line.strip_prefix("vendor_id"))
        .and_then(|rest| rest.split(':').nth(1))
        .map(str::trim);
    Ok(match vendor {
        Some("GenuineIntel") => Some(Hardware::CpuIntel),
        Some("AuthenticAMD") => Some(Hardware::CpuAmd),
        _ => None,
    })
}

fn gpus(root: &Path) -> Result<BTreeSet<Hardware>> {
    let mut found = BTreeSet::new();
    for device in walk::children(&root.join("sys/bus/pci/devices"))? {
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

fn battery(root: &Path) -> Result<bool> {
    for supply in walk::children(&root.join("sys/class/power_supply"))? {
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

fn read(path: &Path) -> Result<String> {
    Ok(fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?
        .trim()
        .to_owned())
}

#[cfg(test)]
#[path = "../tests/hardware.rs"]
mod tests;
