use super::*;

#[test]
fn a_disk_whose_name_ends_in_a_digit_puts_p_before_the_number() {
    assert_eq!(partition("/dev/nvme0n1", 2), "/dev/nvme0n1p2");
    assert_eq!(partition("/dev/mmcblk0", 1), "/dev/mmcblk0p1");
    assert_eq!(partition("/dev/sda", 3), "/dev/sda3");
    assert_eq!(partition("/dev/vda", 1), "/dev/vda1");
}

#[test]
fn a_layout_reads_its_own_name() {
    for layout in Layout::iter() {
        assert_eq!(layout.name().parse::<Layout>().unwrap(), layout);
    }
}

fn devices(json: &str) -> Vec<Device> {
    serde_json::from_str::<Lsblk>(json).unwrap().blockdevices
}

const DISK: &str = r#"{"blockdevices": [{
    "name": "/dev/sdb", "type": "disk", "size": "16G", "model": "Stick", "ro": false,
    "uuid": "2026-11-01-10-12-34-00", "label": "ARCH_202611", "fstype": "iso9660",
    "partlabel": null, "parttype": null, "partn": null, "mountpoints": [null],
    "children": [{
        "name": "/dev/sdb2", "type": "part", "size": "15M", "model": null, "ro": false,
        "uuid": "A1B2-C3D4", "label": "ARCHISO_EFI", "fstype": "vfat", "partlabel": null,
        "parttype": "c12a7328-f81f-11d2-ba4b-00a0c93ec93b", "partn": 2, "mountpoints": []
    }]
}, {
    "name": "/dev/nvme0n1", "type": "disk", "size": "512G", "model": "SSD", "ro": false,
    "uuid": null, "label": null, "fstype": null, "partlabel": null, "parttype": null,
    "partn": null, "mountpoints": [],
    "children": [{
        "name": "/dev/nvme0n1p2", "type": "part", "size": "500G", "model": null, "ro": false,
        "uuid": "x", "label": null, "fstype": "crypto_LUKS", "partlabel": null,
        "parttype": null, "partn": 2, "mountpoints": [null],
        "children": [{
            "name": "/dev/mapper/old", "type": "crypt", "size": "500G", "model": null,
            "ro": false, "uuid": null, "label": null, "fstype": null, "partlabel": null,
            "parttype": null, "partn": null, "mountpoints": [null]
        }]
    }]
}]}"#;

#[test]
fn the_isos_own_stick_is_found_by_its_uuid_or_label() {
    let disks = devices(DISK);
    let by_uuid = Iso {
        uuids: vec!["2026-11-01-10-12-34-00".into()],
        ..Iso::default()
    };
    assert!(by_uuid.holds(&disks[0]));
    assert!(!by_uuid.holds(&disks[1]));
    let by_label = Iso {
        labels: vec!["ARCHISO_EFI".into()],
        ..Iso::default()
    };
    assert!(by_label.holds(&disks[0]));
}

#[test]
fn a_disk_in_use_is_refused() {
    let disks = devices(DISK);
    assert_eq!(disks[0].busy(), None);
    assert_eq!(
        disks[1].busy().as_deref(),
        Some("crypt is open as /dev/mapper/old")
    );
}

#[test]
fn the_free_space_holds_a_1_gib_boot_and_at_least_20_gib_of_root() {
    for sector in [512, 4096] {
        let (start, boot, root) = (2048, GIB / sector, ROOT_MIN / sector);
        let fits = start + boot + root - 1;
        assert_eq!(boot_end(start, fits, sector), Some(start + boot - 1));
        assert_eq!(boot_end(start, fits - 1, sector), None);
        assert_eq!(boot_end(start, start, sector), None);
    }
}

#[test]
fn the_efi_mount_is_needed_once_its_folder_exists() {
    let root = tempfile::tempdir().unwrap();
    let boot = root.path().join("boot");
    assert_eq!(
        mounts_needed(root.path()),
        [root.path().to_path_buf(), boot.clone()]
    );
    let efi = root.path().join("efi");
    fs::create_dir(&efi).unwrap();
    assert_eq!(
        mounts_needed(root.path()),
        [root.path().to_path_buf(), boot, efi]
    );
}
