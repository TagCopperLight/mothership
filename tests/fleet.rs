use super::*;
use tempfile::TempDir;

const SYSTEM: &str = r#"
packages = ["base"]
services = ["sshd.service"]

[hardware.cpu-intel]
packages = ["intel-ucode"]

[hardware.gxfp5130]
packages = ["fprintd"]
built_packages = ["libfprint-gxfp"]
etc = ["pam.d/sudo"]

[options.dual-boot]
packages = ["os-prober"]
services = ["prober.service"]
"#;

fn checkout(system: &str) -> TempDir {
    let dir = TempDir::new().unwrap();
    let type_dir = dir.path().join("types/workstation");
    fs::create_dir_all(type_dir.join("etc/pam.d")).unwrap();
    fs::write(type_dir.join("etc/pam.d/sudo"), "").unwrap();
    fs::write(type_dir.join("etc/locale.conf"), "").unwrap();
    fs::write(type_dir.join("system.toml"), system).unwrap();
    dir
}

fn load(dir: &TempDir, options: &[&str], hardware: &[Hardware]) -> Result<Fleet> {
    let options: Vec<String> = options.iter().map(|o| o.to_string()).collect();
    let hardware = hardware.iter().copied().collect();
    Fleet::new(dir.path().to_path_buf(), "workstation", &options, &hardware)
}

#[test]
fn merges_the_blocks_of_the_machines_hardware_and_options() {
    let dir = checkout(SYSTEM);
    let fleet = load(
        &dir,
        &["dual-boot"],
        &[Hardware::CpuIntel, Hardware::Gxfp5130],
    )
    .unwrap();
    assert_eq!(
        fleet.system.packages,
        ["base", "intel-ucode", "fprintd", "os-prober"]
    );
    assert_eq!(fleet.system.built_packages, ["libfprint-gxfp"]);
    assert_eq!(fleet.system.services, ["sshd.service", "prober.service"]);
    let etc: Vec<&Path> = fleet.etc.iter().map(PathBuf::as_path).collect();
    assert_eq!(etc, [Path::new("locale.conf"), Path::new("pam.d/sudo")]);
    assert!(fleet.etc_elsewhere.is_empty());
}

#[test]
fn leaves_out_the_blocks_the_machine_lacks() {
    let dir = checkout(SYSTEM);
    let fleet = load(&dir, &[], &[Hardware::CpuIntel]).unwrap();
    assert_eq!(fleet.system.packages, ["base", "intel-ucode"]);
    assert!(fleet.system.built_packages.is_empty());
    let etc: Vec<&Path> = fleet.etc.iter().map(PathBuf::as_path).collect();
    assert_eq!(etc, [Path::new("locale.conf")]);
    assert!(fleet.etc_elsewhere.contains(Path::new("pam.d/sudo")));
}

#[test]
fn hardware_without_a_block_or_an_unknown_option_stops_the_load() {
    let dir = checkout(SYSTEM);
    assert!(load(&dir, &[], &[Hardware::CpuAmd]).is_err());
    assert!(load(&dir, &["nvidia"], &[]).is_err());
}

#[test]
fn a_block_file_must_exist_and_belong_to_one_block() {
    let missing = checkout("[hardware.battery]\netc = [\"power.conf\"]\n");
    assert!(load(&missing, &[], &[]).is_err());
    let twice = checkout(
        "[hardware.battery]\netc = [\"pam.d/sudo\"]\n[options.a]\netc = [\"pam.d/sudo\"]\n",
    );
    assert!(load(&twice, &[], &[]).is_err());
}

#[test]
fn an_unknown_hardware_name_is_a_parse_error() {
    let dir = checkout("[hardware.cpu-arm]\npackages = []\n");
    assert!(load(&dir, &[], &[]).is_err());
}
