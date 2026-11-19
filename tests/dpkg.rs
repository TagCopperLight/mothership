use super::*;

#[test]
fn reads_names_from_alternatives_without_versions_or_architectures() {
    let field = "libc6 (>= 2.38), debconf (>= 0.5) | debconf-2.0, python3:any";
    assert_eq!(names(field), ["libc6", "debconf", "debconf-2.0", "python3"]);
    assert!(names("").is_empty());
}

#[test]
fn joins_continuation_lines_into_their_field() {
    let status = "Package: a\nStatus: install ok installed\nConffiles:\n /etc/a 0123\n /etc/b 4567 obsolete\nDescription: first\n more\n\nPackage: b\nStatus: deinstall ok config-files\n";
    let stanzas = stanzas(status).unwrap();
    assert_eq!(stanzas.len(), 2);
    assert_eq!(
        stanzas[0].field("Conffiles"),
        "\n/etc/a 0123\n/etc/b 4567 obsolete"
    );
    assert_eq!(stanzas[0].field("Description"), "first\nmore");
    assert_eq!(stanzas[1].field("Status"), "deinstall ok config-files");
    assert_eq!(stanzas[1].field("Depends"), "");
}

#[test]
fn reads_which_packages_apt_installed_as_dependencies() {
    let states = "Package: a\nArchitecture: amd64\nAuto-Installed: 1\n\nPackage: b\nArchitecture: amd64\nAuto-Installed: 0\n";
    let auto = auto_installed(states).unwrap();
    assert_eq!(auto, BTreeSet::from([("a".to_owned(), "amd64".to_owned())]));
    assert!(is_auto(&auto, "a", "amd64"));
    assert!(is_auto(&auto, "a", "all"));
    assert!(!is_auto(&auto, "a", "i386"));
    assert!(!is_auto(&auto, "b", "amd64"));
}

#[test]
fn reads_the_config_files_ucf_keeps() {
    let registry = "openssh-server \t /etc/ssh/sshd_config\ngone \t /etc/gone.conf\n";
    let hashes = "0123abcd  /etc/ssh/sshd_config\n4567  /etc/with space.conf\n";
    let found = ucf(registry, hashes).unwrap();
    assert_eq!(found.len(), 1);
    assert!(ucf("openssh-server /etc/x\n", hashes).is_err());
    let (name, backup) = &found[0];
    assert_eq!(name, "openssh-server");
    assert_eq!(backup.path, PathBuf::from("/etc/ssh/sshd_config"));
    assert_eq!(backup.md5, "0123abcd");
    assert_eq!(backup.origin, Origin::Script);
}

#[test]
fn reads_the_secondary_links_of_an_alternative_dpkg_picks() {
    let auto = "auto\n/usr/bin/awk\nawk.1.gz\n/usr/share/man/man1/awk.1.gz\nnawk\n/usr/bin/nawk\n\n/usr/bin/mawk\n5\n";
    assert_eq!(picked_by_dpkg(auto), Some(vec!["awk.1.gz", "nawk"]));
    assert_eq!(picked_by_dpkg(&auto.replacen("auto", "manual", 1)), None);
}

#[test]
fn a_line_that_is_no_field_fails() {
    assert!(stanzas("Package: a\nnot a field\n").is_err());
}

#[test]
fn reads_config_files_with_spaces_and_skips_retired_ones() {
    assert_eq!(
        conffile("/etc/a b.conf 0123").unwrap(),
        Some(("/etc/a b.conf", "0123"))
    );
    assert_eq!(conffile("/etc/old.conf 0123 obsolete").unwrap(), None);
    assert_eq!(
        conffile("/etc/old.conf 0123 remove-on-upgrade").unwrap(),
        None
    );
    assert_eq!(conffile("/etc/new.conf newconffile").unwrap(), None);
    assert!(conffile("0123").is_err());
    assert!(conffile("etc/a 0123").is_err());
}

#[test]
fn a_diverted_file_moves_for_every_package_but_the_one_diverting_it() {
    let text = "/etc/issue\n/etc/issue.distrib\nbranding\n/etc/motd\n/etc/motd.orig\n:\n";
    let diversions = Diversions::parse(text).unwrap();
    assert_eq!(
        diversions.of("/etc/issue", "base-files"),
        "/etc/issue.distrib"
    );
    assert_eq!(diversions.of("/etc/issue", "branding"), "/etc/issue");
    assert_eq!(diversions.of("/etc/motd", "base-files"), "/etc/motd.orig");
    assert_eq!(diversions.of("/etc/hosts", "base-files"), "/etc/hosts");
    assert!(Diversions::parse("/etc/issue\n/etc/issue.distrib\n").is_err());
}

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
