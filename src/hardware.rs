use crate::walk;
use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, de};
use std::{collections::BTreeSet, fs, path::Path};
use strum::{EnumString, IntoStaticStr};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, IntoStaticStr, EnumString)]
#[strum(serialize_all = "kebab-case")]
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
    pub fn name(self) -> &'static str {
        self.into()
    }
}

impl<'de> Deserialize<'de> for Hardware {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        name.parse()
            .map_err(|_| de::Error::custom(format!("mothership can't detect {name}")))
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
        match walk::read_if_any(&supply.join("scope"))? {
            Some(scope) if scope.trim() == "Device" => continue,
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
