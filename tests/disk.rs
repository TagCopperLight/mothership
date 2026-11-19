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
    for layout in Layout::ALL {
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
