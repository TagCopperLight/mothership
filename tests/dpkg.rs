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
