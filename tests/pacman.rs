use super::*;

fn package(name: &str, depends: &[&str], provides: &[&str]) -> Package {
    Package {
        name: name.into(),
        version: "1".into(),
        arch: "any".into(),
        description: String::new(),
        explicit: false,
        files: Vec::new(),
        backups: Vec::new(),
        depends: depends.iter().map(|d| d.to_string()).collect(),
        provides: provides.iter().map(|p| p.to_string()).collect(),
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
fn reads_names_without_their_version() {
    let desc = "%DEPENDS%\nglibc>=2.40\nsh\nzlib=1.3\n\n%PROVIDES%\nlibfoo.so=1-64\n";
    assert_eq!(names(desc, "DEPENDS"), ["glibc", "sh", "zlib"]);
    assert_eq!(names(desc, "PROVIDES"), ["libfoo.so"]);
}
