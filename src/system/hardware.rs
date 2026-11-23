use crate::{util::file, util::walk};
use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, de};
use std::{collections::BTreeSet, fs, path::Path};

#[derive(Default, Debug, PartialEq)]
pub struct Hardware {
    pub cpu: Option<String>,
    pub gpus: BTreeSet<u16>,
    pub acpi: BTreeSet<String>,
    pub battery: bool,
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Detect {
    Cpu(String),
    Gpu(#[serde(deserialize_with = "pci_id")] u16),
    Acpi(String),
    Battery,
}

impl Detect {
    pub fn matches(&self, hardware: &Hardware) -> bool {
        match self {
            Detect::Cpu(vendor) => hardware.cpu.as_ref() == Some(vendor),
            Detect::Gpu(vendor) => hardware.gpus.contains(vendor),
            Detect::Acpi(id) => hardware.acpi.contains(id),
            Detect::Battery => hardware.battery,
        }
    }
}

fn pci_id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u16, D::Error> {
    let text = String::deserialize(deserializer)?;
    match u16::from_str_radix(&text, 16) {
        Ok(id) if text.len() == 4 => Ok(id),
        _ => Err(de::Error::custom(format!(
            "{text:?} isn't a PCI vendor id, four hex digits such as \"10de\""
        ))),
    }
}

pub fn detect(root: &Path) -> Result<Hardware> {
    Ok(Hardware {
        cpu: cpu(root)?,
        gpus: gpus(root)?,
        acpi: walk::children(&root.join("sys/bus/acpi/devices"))?
            .iter()
            .filter_map(|path| path.file_name()?.to_str()?.split(':').next())
            .map(str::to_owned)
            .collect(),
        battery: battery(root)?,
    })
}

fn cpu(root: &Path) -> Result<Option<String>> {
    let path = root.join("proc/cpuinfo");
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    Ok(text
        .lines()
        .find_map(|line| line.strip_prefix("vendor_id"))
        .and_then(|rest| rest.split(':').nth(1))
        .map(|vendor| vendor.trim().to_owned()))
}

fn gpus(root: &Path) -> Result<BTreeSet<u16>> {
    let mut found = BTreeSet::new();
    for device in walk::children(&root.join("sys/bus/pci/devices"))? {
        if !read(&device.join("class"))?.starts_with("0x03") {
            continue;
        }
        let path = device.join("vendor");
        let vendor = read(&path)?;
        found.insert(
            vendor
                .strip_prefix("0x")
                .and_then(|hex| u16::from_str_radix(hex, 16).ok())
                .with_context(|| format!("{} holds {vendor:?}", path.display()))?,
        );
    }
    Ok(found)
}

fn battery(root: &Path) -> Result<bool> {
    for supply in walk::children(&root.join("sys/class/power_supply"))? {
        if read(&supply.join("type"))? != "Battery" {
            continue;
        }
        match file::read_if_any(&supply.join("scope"))? {
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
#[path = "../../tests/system/hardware.rs"]
mod tests;
