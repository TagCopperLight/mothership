use super::*;

fn package(name: &str, depends: &[&str], provides: &[&str]) -> Package {
    Package {
        name: name.into(),
        description: String::new(),
        explicit: false,
        files: Vec::new(),
        backups: Vec::new(),
        depends: depends.iter().map(|d| d.to_string()).collect(),
        provides: provides.iter().map(|p| p.to_string()).collect(),
        archive: Archive {
            path: PathBuf::new(),
            distro: Distro::Arch,
        },
    }
}

#[test]
fn follows_dependencies_through_what_packages_provide() {
    let db = Db {
        packages: vec![
            package("base", &["systemd", "sh"], &[]),
            package("systemd", &[], &[]),
            package("bash", &[], &["sh"]),
            package("firefox", &[], &[]),
        ],
    };
    let listed = BTreeSet::from(["base".to_owned()]);
    let found: Vec<String> = db.with_depends(&listed).into_iter().collect();
    assert_eq!(found, ["base", "bash", "systemd"]);
}

#[test]
fn knows_the_family_from_os_release() {
    let arch = "NAME=\"Arch Linux\"\nID=arch\n";
    let debian = "PRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"\nID=debian\n";
    let ubuntu = "ID=ubuntu\nID_LIKE=debian\n";
    let fedora = "ID=fedora\n";
    assert_eq!(Distro::of(arch).unwrap(), Distro::Arch);
    assert_eq!(Distro::of(debian).unwrap(), Distro::Debian);
    assert_eq!(Distro::of(ubuntu).unwrap(), Distro::Debian);
    assert!(Distro::of(fedora).is_err());
}
