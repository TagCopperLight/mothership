use super::*;
use tempfile::TempDir;

struct Root(TempDir);

impl Root {
    fn new(cpu: &str) -> Self {
        let root = Root(TempDir::new().unwrap());
        root.write(
            "proc/cpuinfo",
            &format!("processor\t: 0\nvendor_id\t: {cpu}\n"),
        );
        root
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.0.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("{text}\n")).unwrap();
    }

    fn pci(&self, slot: &str, class: &str, vendor: &str) {
        self.write(&format!("sys/bus/pci/devices/{slot}/class"), class);
        self.write(&format!("sys/bus/pci/devices/{slot}/vendor"), vendor);
    }

    fn detect(&self) -> Hardware {
        detect(self.0.path()).unwrap()
    }
}

#[test]
fn finds_the_laptop() {
    let root = Root::new("GenuineIntel");
    root.pci("0000:00:02.0", "0x030000", "0x8086");
    root.pci("0000:00:1f.3", "0x040100", "0x8086");
    root.write("sys/class/power_supply/AC0/type", "Mains");
    root.write("sys/class/power_supply/BAT0/type", "Battery");
    fs::create_dir_all(root.0.path().join("sys/bus/acpi/devices/GXFP5130:00")).unwrap();
    assert_eq!(
        root.detect(),
        Hardware {
            cpu: Some("GenuineIntel".into()),
            gpus: BTreeSet::from([0x8086]),
            acpi: BTreeSet::from(["GXFP5130".into()]),
            battery: true,
        }
    );
}

#[test]
fn finds_the_desktop_and_skips_a_headset_battery() {
    let root = Root::new("AuthenticAMD");
    root.pci("0000:03:00.0", "0x030000", "0x1002");
    root.write("sys/class/power_supply/hid-headset/type", "Battery");
    root.write("sys/class/power_supply/hid-headset/scope", "Device");
    let found = root.detect();
    assert_eq!(found.cpu.as_deref(), Some("AuthenticAMD"));
    assert_eq!(found.gpus, BTreeSet::from([0x1002]));
    assert!(!found.battery);
}

#[test]
fn skips_devices_that_arent_displays() {
    let root = Root::new("GenuineIntel");
    root.pci("0000:00:1f.3", "0x040100", "0x10de");
    assert!(root.detect().gpus.is_empty());
}

#[test]
fn an_arm_cpu_has_no_vendor() {
    let root = Root::new("GenuineIntel");
    root.write("proc/cpuinfo", "processor\t: 0\nCPU implementer\t: 0x41");
    assert_eq!(root.detect().cpu, None);
}

#[derive(Deserialize)]
struct Block {
    detect: Detect,
}

fn parse(toml: &str) -> Result<Detect, String> {
    toml::from_str::<Block>(toml)
        .map(|block| block.detect)
        .map_err(|e| e.to_string())
}

#[test]
fn each_check_reads_from_the_fleet() {
    assert_eq!(
        parse(r#"detect = { cpu = "AuthenticAMD" }"#),
        Ok(Detect::Cpu("AuthenticAMD".into()))
    );
    assert_eq!(
        parse(r#"detect = { gpu = "10de" }"#),
        Ok(Detect::Gpu(0x10de))
    );
    assert_eq!(
        parse(r#"detect = { acpi = "GXFP5130" }"#),
        Ok(Detect::Acpi("GXFP5130".into()))
    );
    assert_eq!(parse(r#"detect = "battery""#), Ok(Detect::Battery));
    for wrong in ["0x10de", "10d", "nvidia"] {
        let error = parse(&format!("detect = {{ gpu = {wrong:?} }}")).unwrap_err();
        assert!(error.contains("isn't a PCI vendor id"), "{error}");
    }
}

#[test]
fn a_check_matches_only_what_the_machine_has() {
    let laptop = Hardware {
        cpu: Some("GenuineIntel".into()),
        gpus: BTreeSet::from([0x8086]),
        acpi: BTreeSet::from(["GXFP5130".into()]),
        battery: true,
    };
    for detect in [
        Detect::Cpu("GenuineIntel".into()),
        Detect::Gpu(0x8086),
        Detect::Acpi("GXFP5130".into()),
        Detect::Battery,
    ] {
        assert!(detect.matches(&laptop), "{detect:?}");
        assert!(!detect.matches(&Hardware::default()), "{detect:?}");
    }
    assert!(!Detect::Cpu("AuthenticAMD".into()).matches(&laptop));
    assert!(!Detect::Gpu(0x10de).matches(&laptop));
}
