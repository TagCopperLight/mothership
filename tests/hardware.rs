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

    fn detect(&self) -> Vec<Hardware> {
        detect(self.0.path()).unwrap().into_iter().collect()
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
        [
            Hardware::CpuIntel,
            Hardware::GpuIntel,
            Hardware::Battery,
            Hardware::Gxfp5130
        ]
    );
}

#[test]
fn finds_the_desktop_and_skips_a_headset_battery() {
    let root = Root::new("AuthenticAMD");
    root.pci("0000:03:00.0", "0x030000", "0x1002");
    root.write("sys/class/power_supply/hid-headset/type", "Battery");
    root.write("sys/class/power_supply/hid-headset/scope", "Device");
    assert_eq!(root.detect(), [Hardware::CpuAmd, Hardware::GpuAmd]);
}

#[test]
fn a_virtual_display_needs_nothing() {
    let root = Root::new("GenuineIntel");
    root.pci("0000:00:08.0", "0x030000", "0x1414");
    assert_eq!(root.detect(), [Hardware::CpuIntel]);
}

#[test]
fn a_cpu_from_another_vendor_needs_nothing() {
    let root = Root::new("CentaurHauls");
    assert_eq!(root.detect(), []);
}

#[test]
fn an_arm_cpu_needs_nothing() {
    let root = Root::new("GenuineIntel");
    root.write("proc/cpuinfo", "processor\t: 0\nCPU implementer\t: 0x41");
    assert_eq!(root.detect(), []);
}
