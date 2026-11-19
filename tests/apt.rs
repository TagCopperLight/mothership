use super::*;
use crate::package::{Archive, Distro, Package};
use std::path::PathBuf;

fn package(name: &str, required: bool, files: &[&str]) -> Package {
    Package {
        name: name.into(),
        description: String::new(),
        explicit: true,
        required,
        files: files.iter().map(|f| f.to_string()).collect(),
        backups: Vec::new(),
        depends: Vec::new(),
        provides: Vec::new(),
        archive: Archive {
            path: PathBuf::new(),
            distro: Distro::Debian,
        },
    }
}

#[test]
fn the_machine_needs_required_packages_and_those_owning_boot_files_or_enabled_units() {
    let db = Db::new(
        Distro::Debian,
        vec![
            package("apt", true, &["/usr/bin/apt"]),
            package("gdm3", false, &["/usr/lib/systemd/system/gdm3.service"]),
            package(
                "cryptsetup-initramfs",
                false,
                &["/usr/share/initramfs-tools/hooks/cryptroot"],
            ),
            package(
                "firmware-realtek",
                false,
                &["/usr/lib/firmware/", "/usr/lib/firmware/rtl"],
            ),
            package(
                "wpasupplicant",
                false,
                &["/etc/network/if-up.d/wpasupplicant"],
            ),
            package("htop", false, &["/usr/bin/htop", "/usr/lib/firmware/"]),
        ],
    );
    let units = BTreeSet::from(["/usr/lib/systemd/system/gdm3.service".to_owned()]);
    let found: Vec<String> = needed(&db, &units).into_iter().collect();
    assert_eq!(
        found,
        [
            "apt",
            "cryptsetup-initramfs",
            "firmware-realtek",
            "gdm3",
            "wpasupplicant"
        ]
    );
}
