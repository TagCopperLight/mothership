use super::*;
use std::path::PathBuf;

fn vm(options: &[&str]) -> Repo {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("e2e/vm/fleet");
    let options: Vec<String> = options.iter().map(|o| o.to_string()).collect();
    Repo::new(dir, "vm", &options, &Default::default()).unwrap()
}

#[test]
fn the_test_vm_has_what_the_encrypted_disk_needs() {
    let fleet = vm(&["encrypted"]);
    check_type(&fleet).unwrap();
    assert_eq!(keymap(&fleet).unwrap().as_deref(), Some("us"));
}

#[test]
fn a_type_missing_the_boot_pieces_stops_before_the_disk() {
    let error = check_type(&vm(&[])).unwrap_err().to_string();
    assert_eq!(
        error,
        "the encrypted disk needs, from vm and its options: the package sbctl, the package \
         tpm2-tss, /etc/kernel/cmdline, a mkinitcpio preset that builds a UKI in \
         /boot/EFI/Linux, the mkinitcpio hook systemd, the mkinitcpio hook sd-encrypt, the \
         mkinitcpio hook sd-vconsole"
    );
}

#[test]
fn a_root_is_measured_once_pcr_15_holds_something() {
    let zero = "0".repeat(64);
    assert!(!root_measured(&format!("{zero}\n")));
    assert!(root_measured(&format!("{}1\n", &zero[1..])));
}
