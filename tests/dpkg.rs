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
    let stanzas: Vec<Stanza> = stanzas(status).collect();
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
    let auto = auto_installed(states);
    assert_eq!(auto, BTreeSet::from([("a".to_owned(), "amd64".to_owned())]));
    assert!(is_auto(&auto, "a", "amd64"));
    assert!(is_auto(&auto, "a", "all"));
    assert!(!is_auto(&auto, "a", "i386"));
    assert!(!is_auto(&auto, "b", "amd64"));
}

#[test]
fn reads_the_config_files_ucf_keeps() {
    let registry = "openssh-server \t /etc/ssh/sshd_config\ngone \t /etc/gone.conf\n";
    let hashes = "0123abcd  /etc/ssh/sshd_config\n";
    let found = ucf(registry, hashes);
    assert_eq!(found.len(), 1);
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
