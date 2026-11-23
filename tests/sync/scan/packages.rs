use super::*;
use crate::package::{Archive, Distro, Package};
use std::path::PathBuf;

fn package(name: &str, provides: &[&str]) -> Package {
    Package {
        name: name.into(),
        description: String::new(),
        explicit: true,
        required: false,
        files: Vec::new(),
        backups: Vec::new(),
        depends: Vec::new(),
        provides: provides.iter().map(|p| p.to_string()).collect(),
        groups: Vec::new(),
        archive: Archive {
            path: PathBuf::new(),
            distro: Distro::Arch,
        },
    }
}

#[test]
fn a_virtual_name_is_refused_with_the_package_behind_it() {
    let db = Db::new(
        Distro::Arch,
        vec![package("jdk-openjdk", &["java-runtime"])],
    );
    let listed = |name: &str| BTreeSet::from([name.to_owned()]);
    refuse_provided(&db, &listed("jdk-openjdk")).unwrap();
    refuse_provided(&db, &listed("not-installed")).unwrap();
    let err = refuse_provided(&db, &listed("java-runtime")).unwrap_err();
    assert_eq!(
        err.to_string(),
        "java-runtime isn't a package, jdk-openjdk provides it: list jdk-openjdk instead"
    );
}
