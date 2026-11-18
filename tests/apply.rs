use super::*;
use crate::{
    fleet::System,
    ignore::Ignore,
    pacman::{self, Package},
};
use std::path::PathBuf;
use tempfile::TempDir;

fn md5(text: &str) -> String {
    pacman::md5(text.as_bytes())
}

/// A machine whose /etc lives in a temporary folder.
struct Machine {
    dir: TempDir,
}

impl Machine {
    fn new() -> Self {
        Machine {
            dir: TempDir::new().unwrap(),
        }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    fn write(&self, rel: &str, text: &str) -> PathBuf {
        let path = self.path(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }
}

fn no_packages() -> Db {
    Db {
        packages: Vec::new(),
    }
}

fn shipping(path: &Path, text: &str) -> Db {
    Db {
        packages: vec![Package {
            name: "pkg".into(),
            version: "1".into(),
            arch: "any".into(),
            description: String::new(),
            explicit: true,
            files: Vec::new(),
            backups: vec![(path.to_path_buf(), md5(text))],
        }],
    }
}

fn record(edit: impl FnOnce(&mut Record)) -> Record {
    let mut record = Record::default();
    edit(&mut record);
    record
}

fn changed(record: &Record, revert: Revert) -> bool {
    record.fleet_changed(&revert, &no_packages()).unwrap()
}

#[test]
fn installs_a_newly_listed_package_but_not_one_i_removed_myself() {
    let empty = Record::default();
    let listed = record(|r| {
        r.packages.insert("htop".into());
    });
    assert!(changed(&empty, Revert::Install("htop".into())));
    assert!(!changed(&listed, Revert::Install("htop".into())));
}

#[test]
fn uninstalls_only_what_the_fleet_listed_before() {
    let listed = record(|r| {
        r.packages.insert("htop".into());
    });
    assert!(changed(&listed, Revert::Uninstall("htop".into())));
    assert!(!changed(&listed, Revert::Uninstall("firefox".into())));
    assert!(!changed(
        &Record::default(),
        Revert::Uninstall("htop".into())
    ));
}

#[test]
fn services_groups_and_shell_follow_the_same_rule() {
    let before = record(|r| {
        r.services.insert("sshd.service".into());
        r.groups.insert("docker".into());
        r.shell = Some("/usr/bin/fish".into());
    });
    let enable = |unit: &str, user| Revert::Enable {
        unit: unit.into(),
        user,
    };
    let disable = |unit: &str, user| Revert::Disable {
        unit: unit.into(),
        user,
    };
    assert!(changed(&before, enable("new.service", false)));
    assert!(!changed(&before, enable("sshd.service", false)));
    assert!(changed(&before, disable("sshd.service", false)));
    assert!(!changed(&before, disable("mine.service", false)));
    // User units have a list of their own.
    assert!(!changed(&before, disable("sshd.service", true)));
    assert!(changed(&before, enable("sshd.service", true)));

    assert!(changed(&before, Revert::Join("video".into())));
    assert!(!changed(&before, Revert::Join("docker".into())));
    assert!(changed(&before, Revert::Leave("docker".into())));
    assert!(!changed(&before, Revert::Leave("audio".into())));

    assert!(!changed(&before, Revert::Shell("/usr/bin/fish".into())));
    assert!(changed(&before, Revert::Shell("/usr/bin/zsh".into())));
}

#[test]
fn writes_a_missing_file_unless_i_deleted_it() {
    let machine = Machine::new();
    let path = machine.path("new.conf");
    assert!(changed(&Record::default(), Revert::WriteEtc(path.clone())));
    let written = record(|r| {
        r.etc.insert(path.clone(), md5("old"));
    });
    assert!(!changed(&written, Revert::WriteEtc(path)));
}

#[test]
fn writes_over_a_file_only_while_it_is_as_apply_or_its_package_left_it() {
    let machine = Machine::new();
    let path = machine.write("a.conf", "old");
    let as_written = record(|r| {
        r.etc.insert(path.clone(), md5("old"));
    });
    assert!(changed(&as_written, Revert::WriteEtc(path.clone())));

    let edited = record(|r| {
        r.etc.insert(path.clone(), md5("something else"));
    });
    assert!(!changed(&edited, Revert::WriteEtc(path.clone())));
    assert!(!changed(&Record::default(), Revert::WriteEtc(path.clone())));

    let from_package = Record::default()
        .fleet_changed(&Revert::WriteEtc(path.clone()), &shipping(&path, "old"))
        .unwrap();
    assert!(from_package);
    let edited_package_file = Record::default()
        .fleet_changed(&Revert::WriteEtc(path.clone()), &shipping(&path, "new"))
        .unwrap();
    assert!(!edited_package_file);
}

#[test]
fn deletes_or_restores_a_dropped_file_only_when_untouched() {
    let machine = Machine::new();
    let path = machine.write("a.conf", "mine");
    let untouched = record(|r| {
        r.etc.insert(path.clone(), md5("mine"));
    });
    let edited = record(|r| {
        r.etc.insert(path.clone(), md5("what apply wrote"));
    });
    let delete = || Revert::DeleteEtc(path.clone());
    let restore = || Revert::RestoreEtc {
        path: path.clone(),
        archive: PathBuf::new(),
    };
    assert!(changed(&untouched, delete()));
    assert!(changed(&untouched, restore()));
    assert!(!changed(&edited, delete()));
    assert!(!changed(&edited, restore()));
    assert!(!changed(&Record::default(), delete()));
    assert!(!changed(&Record::default(), restore()));
}

#[test]
fn deletes_a_dropped_folder_only_when_apply_wrote_all_of_it() {
    let machine = Machine::new();
    let a = machine.write("unit.d/a.conf", "a");
    let b = machine.write("unit.d/b.conf", "b");
    let folder = machine.path("unit.d");
    let both = record(|r| {
        r.etc.insert(a.clone(), md5("a"));
        r.etc.insert(b.clone(), md5("b"));
    });
    assert!(changed(&both, Revert::DeleteEtc(folder.clone())));

    let one = record(|r| {
        r.etc.insert(a.clone(), md5("a"));
    });
    assert!(!changed(&one, Revert::DeleteEtc(folder.clone())));

    assert!(!changed(&Record::default(), Revert::DeleteEtc(folder)));
}

#[test]
fn never_reads_a_folder_apply_did_not_write_into() {
    use std::os::unix::fs::PermissionsExt;
    let machine = Machine::new();
    machine.write("credstore/secret", "key");
    let folder = machine.path("credstore");
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o000)).unwrap();
    let elsewhere = record(|r| {
        r.etc.insert(machine.path("other.conf"), md5("a"));
    });
    let result = elsewhere.fleet_changed(&Revert::DeleteEtc(folder.clone()), &no_packages());
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!result.unwrap());
}

#[test]
fn applies_chezmoi_files_unless_edited_here() {
    let file = |edited| Revert::ApplyFile {
        path: PathBuf::from("/home/me/.bashrc"),
        edited,
    };
    assert!(changed(&Record::default(), file(false)));
    assert!(!changed(&Record::default(), file(true)));
}

#[test]
fn a_symlink_is_hashed_by_its_target() {
    let machine = Machine::new();
    let link = machine.path("resolv.conf");
    std::os::unix::fs::symlink("/run/stub", &link).unwrap();
    assert_eq!(hash(&link).unwrap(), Some(md5("/run/stub")));
    assert_eq!(hash(&machine.path("missing")).unwrap(), None);
}

fn fleet(dir: &Path, system: &str) -> Fleet {
    let type_dir = dir.join("types/workstation");
    fs::create_dir_all(type_dir.join("etc")).unwrap();
    fs::create_dir_all(dir.join("packages")).unwrap();
    fs::write(dir.join("packages/base.txt"), "vim zip\n").unwrap();
    fs::write(dir.join("packages/local.txt"), "mytool\n").unwrap();
    let system: System = toml::from_str(system).unwrap();
    Fleet {
        dir: dir.to_path_buf(),
        type_dir: type_dir.clone(),
        system,
        ignore: Ignore::load(&type_dir.join("ignore.txt")).unwrap(),
    }
}

#[test]
fn the_new_record_holds_what_the_fleet_declares_and_keeps_edits_as_edits() {
    let checkout = TempDir::new().unwrap();
    let fleet = fleet(
        checkout.path(),
        r#"
packages = ["base"]
built_packages = ["local"]
shell = "/usr/bin/fish"
groups = ["docker"]
services = ["sshd.service"]
user_services = ["app.service"]
"#,
    );
    let etc = fleet.etc_dir();
    fs::write(etc.join("same.conf"), "fleet").unwrap();
    fs::write(etc.join("edited.conf"), "fleet").unwrap();
    fs::write(etc.join("new.conf"), "fleet").unwrap();

    let machine = Machine::new();
    machine.write("same.conf", "fleet");
    machine.write("edited.conf", "mine");
    machine.write("new.conf", "mine");
    machine.write("dropped.conf", "old");
    let before = record(|r| {
        r.packages.insert("dropped".into());
        r.etc.insert(machine.path("edited.conf"), md5("older"));
        r.etc.insert(machine.path("dropped.conf"), md5("old"));
    });

    let after = Record::after(&fleet, &before, machine.dir.path()).unwrap();
    let names = |set: &BTreeSet<String>| set.iter().cloned().collect::<Vec<_>>();
    // Built packages come from elsewhere, so apply never removes them.
    assert_eq!(names(&after.packages), ["vim", "zip"]);
    assert_eq!(names(&after.groups), ["docker"]);
    assert_eq!(names(&after.services), ["sshd.service"]);
    assert_eq!(names(&after.user_services), ["app.service"]);
    assert_eq!(after.shell.as_deref(), Some("/usr/bin/fish"));
    let expected = BTreeMap::from([
        (machine.path("same.conf"), md5("fleet")),
        (machine.path("edited.conf"), md5("older")),
    ]);
    assert_eq!(after.etc, expected);
}

#[test]
fn the_record_survives_a_save_and_starts_empty() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state/applied.json");
    assert!(Record::load(&path).unwrap().packages.is_empty());
    let saved = record(|r| {
        r.packages.insert("vim".into());
        r.etc.insert(PathBuf::from("/etc/a"), md5("a"));
    });
    saved.save(&path).unwrap();
    let loaded = Record::load(&path).unwrap();
    assert_eq!(loaded.packages, saved.packages);
    assert_eq!(loaded.etc, saved.etc);
}
