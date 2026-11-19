use super::disk;
use crate::{
    cmd::{self, As},
    file,
    fleet::Repo,
};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::Path,
    process,
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::Duration,
};

pub const BINARY: &str = "/usr/local/bin/mothership";
const EFI_GLOBAL: &str = "8be4df61-93ca-11d2-aa0d-00e098032b8c";
const UKIS: &str = "/boot/EFI/Linux";
const SYSTEMD_BOOT: &str = "/usr/lib/systemd/boot/efi/systemd-bootx64.efi";
const SERVICE: &str = "mothership-install.service";
const SERVICE_DIR: &str = "/etc/systemd/system";
const CMDLINE: &str = "kernel/cmdline";
const VCONSOLE: &str = "vconsole.conf";
const PACKAGES: [&str; 3] = ["linux", "sbctl", "tpm2-tss"];
const HOOKS: [&str; 2] = ["systemd", "sd-encrypt"];
const KEYMAP_HOOK: &str = "sd-vconsole";
const PCRS: &str = "7+15:sha256=0000000000000000000000000000000000000000000000000000000000000000";
const ANSWER_WITHIN: Duration = Duration::from_secs(30 * 60);

fn unit() -> String {
    format!(
        "[Unit]\n\
         Description=Finish the mothership install\n\
         Wants=network-online.target getty-pre.target\n\
         After=network-online.target systemd-user-sessions.service\n\
         Before=getty-pre.target display-manager.service\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         TimeoutStartSec=infinity\n\
         ExecStart={BINARY} install\n\
         StandardInput=tty\n\
         StandardOutput=tty\n\
         TTYPath=/dev/console\n\
         TTYReset=yes\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n"
    )
}

fn efi_var(name: &str) -> Result<Option<u8>> {
    let path = format!("/sys/firmware/efi/efivars/{name}-{EFI_GLOBAL}");
    let bytes = file::if_found(fs::read(&path)).with_context(|| format!("reading {path}"))?;
    Ok(bytes.and_then(|bytes| bytes.get(4).copied()))
}

pub fn check_firmware() -> Result<()> {
    if !Path::new("/sys/firmware/efi").exists() {
        bail!("the ISO started in BIOS mode: start it in UEFI mode");
    }
    if efi_var("SetupMode")? != Some(1) {
        bail!(
            "Secure Boot isn't in Setup Mode: clear its keys in the firmware setup, then start \
             the ISO again"
        );
    }
    let version = file::read_if_any(Path::new("/sys/class/tpm/tpm0/tpm_version_major"))?;
    if version.as_deref().map(str::trim) != Some("2") {
        bail!("the machine has no TPM 2.0 to unlock the disk");
    }
    Ok(())
}

fn etc_text(fleet: &Repo, rel: &str) -> Result<Option<String>> {
    if !fleet.etc.contains(Path::new(rel)) {
        return Ok(None);
    }
    let path = fleet.etc_source(Path::new(rel));
    fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))
        .map(Some)
}

fn etc_texts(fleet: &Repo, under: &str, suffix: &str) -> Result<Vec<String>> {
    let mut texts = Vec::new();
    for rel in &fleet.etc {
        let rel = rel.to_string_lossy();
        if rel.starts_with(under) && rel.ends_with(suffix) {
            texts.extend(etc_text(fleet, &rel)?);
        }
    }
    Ok(texts)
}

fn value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let value = line.trim().strip_prefix(key)?.strip_prefix('=')?;
    Some(value.trim().trim_matches(['"', '\'']))
}

pub fn keymap(fleet: &Repo) -> Result<Option<String>> {
    Ok(etc_text(fleet, VCONSOLE)?.and_then(|text| {
        text.lines()
            .find_map(|line| value(line, "KEYMAP"))
            .map(str::to_owned)
    }))
}

fn hooks(fleet: &Repo) -> Result<Vec<String>> {
    let configs = etc_texts(fleet, "mkinitcpio.conf", ".conf")?;
    Ok(configs
        .iter()
        .flat_map(|text| text.lines())
        .filter_map(|line| value(line, "HOOKS"))
        .next_back()
        .map(|hooks| {
            hooks
                .trim_matches(['(', ')'])
                .split_whitespace()
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default())
}

fn builds_uki(fleet: &Repo) -> Result<bool> {
    let presets = etc_texts(fleet, "mkinitcpio.d/", ".preset")?;
    Ok(presets.iter().flat_map(|text| text.lines()).any(|line| {
        line.split_once('=').is_some_and(|(key, path)| {
            key.trim().ends_with("_uki")
                && path
                    .trim()
                    .trim_matches(['"', '\''])
                    .starts_with(&format!("{UKIS}/"))
        })
    }))
}

pub fn check_type(fleet: &Repo) -> Result<()> {
    let mut missing = Vec::new();
    let packages = fleet.packages();
    for package in PACKAGES {
        if !packages.contains(package) {
            missing.push(format!("the package {package}"));
        }
    }
    if !fleet.etc.contains(Path::new(CMDLINE)) {
        missing.push(format!("/etc/{CMDLINE}"));
    }
    if !builds_uki(fleet)? {
        missing.push(format!("a mkinitcpio preset that builds a UKI in {UKIS}"));
    }
    let hooks = hooks(fleet)?;
    let keymap_hook = keymap(fleet)?.map(|_| KEYMAP_HOOK);
    for hook in HOOKS.into_iter().chain(keymap_hook) {
        if !hooks.iter().any(|h| h == hook) {
            missing.push(format!("the mkinitcpio hook {hook}"));
        }
    }
    if !missing.is_empty() {
        bail!(
            "the encrypted disk needs, from {} and its options: {}",
            fleet.type_name,
            missing.join(", ")
        );
    }
    Ok(())
}

pub fn setup() -> Result<()> {
    cmd::root("mkinitcpio", &["--allpresets"])?;
    let status: serde_json::Value =
        serde_json::from_str(&cmd::stdout(As::Root, "sbctl", &["status", "--json"])?)?;
    if status["installed"] != true {
        cmd::root("sbctl", &["create-keys"])?;
    }
    if efi_var("SetupMode")? == Some(1) {
        cmd::root("sbctl", &["enroll-keys", "--microsoft"])?;
    }
    let signed = format!("{SYSTEMD_BOOT}.signed");
    cmd::root(
        "sbctl",
        &["sign", "--save", "--output", &signed, SYSTEMD_BOOT],
    )?;
    cmd::root("bootctl", &["install"])?;
    for uki in ukis()? {
        cmd::root("sbctl", &["sign", "--save", &uki])?;
    }

    file::write(
        &Path::new(SERVICE_DIR).join(SERVICE),
        unit().as_bytes(),
        0o644,
    )?;
    cmd::root("systemctl", &["enable", SERVICE])?;
    println!("Secure Boot holds this machine's keys now.");
    Ok(())
}

fn ukis() -> Result<Vec<String>> {
    let mut ukis = Vec::new();
    for entry in fs::read_dir(UKIS).with_context(|| format!("reading {UKIS}"))? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "efi") {
            ukis.push(path.to_string_lossy().into_owned());
        }
    }
    Ok(ukis)
}

pub fn first_start() -> bool {
    Path::new(SERVICE_DIR).join(SERVICE).exists()
}

pub struct Answering {
    _done: mpsc::Sender<()>,
}

pub fn answering() -> Answering {
    let (done, finished) = mpsc::channel::<()>();
    thread::spawn(move || {
        if finished.recv_timeout(ANSWER_WITHIN) == Err(RecvTimeoutError::Timeout) {
            eprintln!(
                "Nobody answered for {} minutes: install stops, and asks again at the next start.",
                ANSWER_WITHIN.as_secs() / 60
            );
            process::exit(1);
        }
    });
    Answering { _done: done }
}

pub fn secure() -> Result<()> {
    if efi_var("SecureBoot")? != Some(1) {
        bail!("Secure Boot is off: turn it on in the firmware setup, then start the machine again");
    }
    let device = luks_device()?;
    let slots = cmd::stdout(As::Root, "systemd-cryptenroll", &[&device])?;
    if !slots.split_whitespace().any(|word| word == "tpm2") {
        println!("The TPM opens the disk from now on. Type the passphrase once more:");
        cmd::root(
            "systemd-cryptenroll",
            &[
                "--tpm2-device=auto",
                &format!("--tpm2-pcrs={PCRS}"),
                &device,
            ],
        )?;
    }
    Ok(())
}

pub fn finish() -> Result<()> {
    cmd::root("systemctl", &["disable", SERVICE])?;
    fs::remove_file(Path::new(SERVICE_DIR).join(SERVICE))?;
    cmd::root("systemctl", &["daemon-reload"])
}

fn luks_device() -> Result<String> {
    let status = cmd::stdout(As::Root, "cryptsetup", &["status", disk::MAPPER])?;
    status
        .lines()
        .find_map(|line| line.trim().strip_prefix("device:"))
        .map(|device| device.trim().to_owned())
        .with_context(|| format!("cryptsetup names no device under {}", disk::MAPPER))
}

#[cfg(test)]
#[path = "../../tests/boot.rs"]
mod tests;
