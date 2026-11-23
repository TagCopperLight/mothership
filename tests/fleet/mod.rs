use super::*;
use tempfile::TempDir;

const SYSTEM: &str = r#"
packages = ["base"]
services = ["sshd.service"]

[hardware.intel]
detect = { cpu = "GenuineIntel" }
packages = ["intel-ucode"]

[hardware.fingerprint]
detect = { acpi = "GXFP5130" }
packages = ["fprintd"]
built_packages = ["libfprint-gxfp"]
etc = ["pam.d/sudo"]

[options.dual-boot]
packages = ["os-prober"]
services = ["prober.service"]
"#;

fn checkout(system: &str) -> TempDir {
    let dir = TempDir::new().unwrap();
    crate::support::name_recovery(dir.path());
    let type_dir = dir.path().join("types/workstation");
    fs::create_dir_all(type_dir.join("etc/pam.d")).unwrap();
    fs::write(type_dir.join("etc/pam.d/sudo"), "").unwrap();
    fs::write(type_dir.join("etc/locale.conf"), "").unwrap();
    fs::write(type_dir.join(SYSTEM_TOML), system).unwrap();
    dir
}

fn intel() -> Hardware {
    Hardware {
        cpu: Some("GenuineIntel".into()),
        ..Hardware::default()
    }
}

fn load(dir: &TempDir, options: &[&str], hardware: &Hardware) -> Result<Repo> {
    let options: Vec<String> = options.iter().map(|o| o.to_string()).collect();
    Repo::new(dir.path().to_path_buf(), "workstation", &options, hardware)
}

fn refused(dir: &TempDir, options: &[&str], hardware: &Hardware, why: &str) {
    let error = format!("{:#}", load(dir, options, hardware).err().unwrap());
    assert!(error.contains(why), "{error:?} doesn't say {why:?}");
}

#[test]
fn merges_the_blocks_of_the_machines_hardware_and_options() {
    let dir = checkout(SYSTEM);
    let laptop = Hardware {
        acpi: BTreeSet::from(["GXFP5130".into()]),
        ..intel()
    };
    let fleet = load(&dir, &["dual-boot"], &laptop).unwrap();
    assert_eq!(
        fleet.system.packages,
        ["base", "fprintd", "intel-ucode", "os-prober"]
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
    let fleet = load(&dir, &[], &intel()).unwrap();
    assert_eq!(fleet.system.packages, ["base", "intel-ucode"]);
    assert!(fleet.system.built_packages.is_empty());
    let etc: Vec<&Path> = fleet.etc.iter().map(PathBuf::as_path).collect();
    assert_eq!(etc, [Path::new("locale.conf")]);
    assert!(fleet.etc_elsewhere.contains(Path::new("pam.d/sudo")));
}

#[test]
fn hardware_no_block_detects_brings_nothing() {
    let dir = checkout(SYSTEM);
    let amd = Hardware {
        cpu: Some("AuthenticAMD".into()),
        gpus: BTreeSet::from([0x10de]),
        ..Hardware::default()
    };
    let fleet = load(&dir, &[], &amd).unwrap();
    assert_eq!(fleet.system.packages, ["base"]);
}

#[test]
fn an_unknown_option_stops_the_load() {
    let dir = checkout(SYSTEM);
    refused(&dir, &["nvidia"], &intel(), "has no [options.nvidia]");
}

#[test]
fn only_hardware_blocks_detect() {
    let silent = checkout("[hardware.intel]\npackages = [\"intel-ucode\"]\n");
    refused(&silent, &[], &intel(), "[hardware.intel] of");
    refused(&silent, &[], &intel(), "says nothing to detect");
    let option = checkout("[options.laptop]\ndetect = \"battery\"\n");
    refused(&option, &[], &intel(), "[options.laptop] of");
    refused(&option, &[], &intel(), "turns it on");
}

#[test]
fn a_block_file_must_exist_and_belong_to_one_block() {
    let missing = checkout("[hardware.laptop]\ndetect = \"battery\"\netc = [\"power.conf\"]\n");
    refused(
        &missing,
        &[],
        &intel(),
        "names etc/power.conf, which isn't there",
    );
    let twice = checkout(
        "[hardware.laptop]\ndetect = \"battery\"\netc = [\"pam.d/sudo\"]\n[options.a]\netc = [\"pam.d/sudo\"]\n",
    );
    refused(&twice, &[], &intel(), "own etc/pam.d/sudo");
}

#[test]
fn each_folder_under_users_is_a_user_of_the_type() {
    let dir = checkout(SYSTEM);
    let users = dir.path().join("types/workstation/users");
    for (name, toml) in [
        ("tag", "shell = \"/usr/bin/fish\"\nlinger = true\n"),
        ("dev", ""),
    ] {
        fs::create_dir_all(users.join(name)).unwrap();
        fs::write(users.join(name).join("user.toml"), toml).unwrap();
    }
    fs::write(users.join("dev/ignore.txt"), "home ~/.cache\n").unwrap();
    let fleet = load(&dir, &[], &intel()).unwrap();
    let names: Vec<&str> = fleet.profiles.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["dev", "tag"]);
    assert!(fleet.profiles[1].config.linger);
    assert_eq!(
        fleet.profiles[1].recipients(),
        users.join("tag/recipients.txt")
    );

    fs::write(users.join("dev/ignore.txt"), "package htop\n").unwrap();
    let cpu = intel();
    refused(&dir, &[], &cpu, "package lines go in the type's ignore.txt");
    fs::write(users.join("dev/ignore.txt"), "").unwrap();
    fs::write(
        dir.path().join("types/workstation/ignore.txt"),
        "home ~/.cache\n",
    )
    .unwrap();
    refused(&dir, &[], &cpu, "home lines go in a user's ignore.txt");
}

#[test]
fn etc_secrets_open_with_root_key_only() {
    let dir = checkout("");
    let list = dir.path().join("list.txt");
    let (root, user) = (
        secrets::make_key(&dir.path().join("root.txt")).unwrap(),
        secrets::make_key(&dir.path().join("user.txt")).unwrap(),
    );
    fs::write(
        &list,
        format!(
            "# root\n{}\n\n# user\n{}\n",
            secrets::public(&root),
            secrets::public(&user)
        ),
    )
    .unwrap();
    let etc = dir.path().join("types/workstation/etc");
    fs::write(
        etc.join("wifi.conf.age"),
        crate::support::seal(&list, b"psk=1"),
    )
    .unwrap();
    let mut fleet = Fleet::of(
        load(&dir, &[], &Hardware::default()).unwrap(),
        &crate::support::current_user(),
    );
    fleet.keys.push(user);
    assert!(fleet.etc_contents(Path::new("wifi.conf")).is_err());
    fleet.root_keys.push(root);
    assert_eq!(
        fleet.etc_contents(Path::new("wifi.conf")).unwrap(),
        b"psk=1"
    );
}
