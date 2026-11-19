use super::*;

fn package(name: &str, depends: &[&str], provides: &[&str]) -> Package {
    Package {
        name: name.into(),
        description: String::new(),
        explicit: false,
        required: false,
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
    let db = Db::new(
        Distro::Arch,
        vec![
            package("base", &["systemd", "sh"], &[]),
            package("systemd", &[], &[]),
            package("bash", &[], &["sh"]),
            package("firefox", &[], &[]),
        ],
    );
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

#[test]
fn finds_every_package_that_leads_to_a_needed_one() {
    let db = Db::new(
        Distro::Debian,
        vec![
            package("linux-image-amd64", &["linux-image-6.12"], &[]),
            package("linux-image-6.12", &[], &[]),
            package("task-laptop", &["linux-image-amd64", "sh"], &[]),
            package("bash", &[], &["sh"]),
            package("htop", &[], &[]),
        ],
    );
    let needed = BTreeSet::from(["linux-image-6.12".to_owned()]);
    let found: Vec<String> = db.needing(&needed).into_iter().collect();
    assert_eq!(
        found,
        ["linux-image-6.12", "linux-image-amd64", "task-laptop"]
    );
    assert_eq!(db.needed_by("sh"), ["task-laptop"]);
    assert_eq!(db.resolve("sh").unwrap().name, "bash");
}

#[test]
fn a_failed_removal_marks_the_packages_again_and_keeps_its_error() {
    let mut restored = false;
    let err = or_restore(Err::<(), _>(anyhow::anyhow!("removal failed")), || {
        restored = true;
        Ok(())
    })
    .unwrap_err();
    assert!(restored);
    assert_eq!(err.to_string(), "removal failed");
    assert_eq!(or_restore(Ok(1), || unreachable!()).unwrap(), 1);
}

#[test]
fn an_installed_name_must_be_a_real_package_name() {
    let db = Db::new(
        Distro::Arch,
        vec![package("jdk-openjdk", &[], &["java-runtime"])],
    );
    db.require_installed(&["jdk-openjdk"]).unwrap();
    let err = db.require_installed(&["java-runtime"]).unwrap_err();
    assert!(err.to_string().starts_with("java-runtime isn't the name"));
    assert!(db.require_installed(&["gnome"]).is_err());
}
