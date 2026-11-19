use super::BINARY;
use crate::cmd::{self, As};
use anyhow::{Context, Result, bail};
use std::{fs, io::ErrorKind, path::Path};

const EFI_GLOBAL: &str = "8be4df61-93ca-11d2-aa0d-00e098032b8c";
const UKIS: &str = "/boot/EFI/Linux";
const SYSTEMD_BOOT: &str = "/usr/lib/systemd/boot/efi/systemd-bootx64.efi";
const SERVICE: &str = "mothership-install.service";
const SERVICE_DIR: &str = "/etc/systemd/system";
const CMDLINE: &str = "/etc/kernel/cmdline";
const PCRS: &str = "7+15:sha256=0000000000000000000000000000000000000000000000000000000000000000";

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
    match fs::read(&path) {
        Ok(bytes) => Ok(bytes.get(4).copied()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {path}")),
    }
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
    let version = match fs::read_to_string("/sys/class/tpm/tpm0/tpm_version_major") {
        Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
        version => version.context("reading the TPM's version")?,
    };
    if version.trim() != "2" {
        bail!("the machine has no TPM 2.0 to unlock the disk");
    }
    Ok(())
}

pub fn setup() -> Result<()> {
    if !Path::new(CMDLINE).exists() {
        bail!("the type has no {CMDLINE}, which the encrypted disk's UKI needs");
    }
    cmd::root("mkinitcpio", &["--allpresets"])?;
    let ukis = ukis()?;
    if ukis.is_empty() {
        bail!(
            "mkinitcpio made no image in {UKIS}: the type's mkinitcpio preset must build a UKI \
             there, or the encrypted disk never opens"
        );
    }
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
    for uki in &ukis {
        cmd::root("sbctl", &["sign", "--save", uki])?;
    }

    fs::write(Path::new(SERVICE_DIR).join(SERVICE), unit())?;
    cmd::root("systemctl", &["enable", SERVICE])?;
    println!("Secure Boot holds this machine's keys now.");
    Ok(())
}

fn ukis() -> Result<Vec<String>> {
    let entries = match fs::read_dir(UKIS) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        entries => entries.with_context(|| format!("reading {UKIS}"))?,
    };
    let mut ukis = Vec::new();
    for entry in entries {
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

pub fn finish() -> Result<()> {
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
    cmd::root("systemctl", &["disable", SERVICE])?;
    fs::remove_file(Path::new(SERVICE_DIR).join(SERVICE))?;
    cmd::root("systemctl", &["daemon-reload"])
}

fn luks_device() -> Result<String> {
    let status = cmd::stdout(As::Root, "cryptsetup", &["status", "root"])?;
    status
        .lines()
        .find_map(|line| line.trim().strip_prefix("device:"))
        .map(|device| device.trim().to_owned())
        .context("cryptsetup names no device under root")
}
