use super::*;
use crate::{
    fleet::{Manager, Tool},
    package::{self, Archive, Backup, Distro, Origin, Package},
    revert,
};
use std::path::PathBuf;
use tempfile::TempDir;

fn md5(text: &str) -> String {
    package::md5(text.as_bytes())
}

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

fn no_archive() -> Archive {
    Archive {
        path: PathBuf::new(),
        distro: Distro::Arch,
    }
}

fn no_packages() -> Db {
    Db::new(Distro::Arch, Vec::new())
}

fn shipping(path: &Path, text: &str) -> Db {
    Db::new(
        Distro::Arch,
        vec![Package {
            name: "pkg".into(),
            description: String::new(),
            explicit: true,
            required: false,
            files: Vec::new(),
            backups: vec![Backup {
                path: path.to_path_buf(),
                md5: md5(text),
                origin: Origin::Archive,
            }],
            depends: Vec::new(),
            provides: Vec::new(),
            archive: no_archive(),
        }],
    )
}

fn record(edit: impl FnOnce(&mut Record)) -> Record {
    let mut record = Record::default();
    edit(&mut record);
    record
}

fn me(record: &mut Record) -> &mut UserRecord {
    record.users.entry("me".into()).or_default()
}

fn changed(record: &Record, revert: impl Into<Revert>) -> bool {
    record
        .fleet_changed(&revert.into(), Some("me"), &no_packages())
        .unwrap()
}

#[test]
fn installs_a_newly_listed_package_but_not_one_i_removed_myself() {
    let empty = Record::default();
    let listed = record(|r| {
        r.packages.insert("htop".into());
    });
    assert!(changed(&empty, revert::Machine::Install("htop".into())));
    assert!(!changed(&listed, revert::Machine::Install("htop".into())));
}

#[test]
fn uninstalls_only_what_the_fleet_listed_before() {
    let listed = record(|r| {
        r.packages.insert("htop".into());
    });
    assert!(changed(&listed, revert::Machine::Uninstall("htop".into())));
    assert!(!changed(
        &listed,
        revert::Machine::Uninstall("firefox".into())
    ));
    assert!(!changed(
        &Record::default(),
        revert::Machine::Uninstall("htop".into())
    ));
}

#[test]
fn services_groups_and_shell_follow_the_same_rule() {
    let before = record(|r| {
        r.services.insert("sshd.service".into());
        me(r).groups.insert("docker".into());
        me(r).shell = Some("/usr/bin/fish".into());
    });
    let enable = |unit: &str| revert::Machine::Unit(Unit::Enable(unit.into()));
    let disable = |unit: &str| revert::Machine::Unit(Unit::Disable(unit.into()));
    assert!(changed(&before, enable("new.service")));
    assert!(!changed(&before, enable("sshd.service")));
    assert!(changed(&before, disable("sshd.service")));
    assert!(!changed(&before, disable("mine.service")));
    assert!(!changed(
        &before,
        Service::Unit(Unit::Disable("sshd.service".into()))
    ));
    assert!(changed(
        &before,
        Service::Unit(Unit::Enable("sshd.service".into()))
    ));

    assert!(changed(&before, Account::Join("video".into())));
    assert!(!changed(&before, Account::Join("docker".into())));
    assert!(changed(&before, Account::Leave("docker".into())));
    assert!(!changed(&before, Account::Leave("audio".into())));

    assert!(!changed(&before, Account::Shell("/usr/bin/fish".into())));
    assert!(changed(&before, Account::Shell("/usr/bin/zsh".into())));
}

#[test]
fn each_user_has_a_record_of_their_own() {
    let before = record(|r| {
        me(r).groups.insert("docker".into());
        me(r).linger = true;
    });
    let other = |revert: Revert| {
        before
            .fleet_changed(&revert, Some("dev"), &no_packages())
            .unwrap()
    };
    assert!(changed(&before, Account::Leave("docker".into())));
    assert!(!other(Account::Leave("docker".into()).into()));
    assert!(changed(&before, Service::Linger(false)));
    assert!(other(Service::Linger(true).into()));
    assert!(changed(&before, User::Create));
    assert!(changed(&before, User::InitChezmoi));
}

#[test]
fn writes_a_missing_file_unless_i_deleted_it() {
    let machine = Machine::new();
    let path = machine.path("new.conf");
    assert!(changed(&Record::default(), Etc::Write(path.clone())));
    let written = record(|r| {
        r.etc.insert(path.clone(), md5("old"));
    });
    assert!(!changed(&written, Etc::Write(path)));
}

#[test]
fn writes_over_a_file_only_while_it_is_as_apply_or_its_package_left_it() {
    let machine = Machine::new();
    let path = machine.write("a.conf", "old");
    let as_written = record(|r| {
        r.etc.insert(path.clone(), md5("old"));
    });
    assert!(changed(&as_written, Etc::Write(path.clone())));

    let edited = record(|r| {
        r.etc.insert(path.clone(), md5("something else"));
    });
    assert!(!changed(&edited, Etc::Write(path.clone())));
    assert!(!changed(&Record::default(), Etc::Write(path.clone())));

    let from_package = Record::default()
        .fleet_changed(
            &Etc::Write(path.clone()).into(),
            None,
            &shipping(&path, "old"),
        )
        .unwrap();
    assert!(from_package);
    let edited_package_file = Record::default()
        .fleet_changed(
            &Etc::Write(path.clone()).into(),
            None,
            &shipping(&path, "new"),
        )
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
    let delete = || Etc::Delete(path.clone());
    let restore = || Etc::Restore {
        path: path.clone(),
        archive: no_archive(),
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
    assert!(changed(&both, Etc::Delete(folder.clone())));

    let one = record(|r| {
        r.etc.insert(a.clone(), md5("a"));
    });
    assert!(!changed(&one, Etc::Delete(folder.clone())));

    assert!(!changed(&Record::default(), Etc::Delete(folder)));
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
    let result = elsewhere.fleet_changed(&Etc::Delete(folder.clone()).into(), None, &no_packages());
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!result.unwrap());
}

#[test]
fn applies_chezmoi_files_unless_edited_here() {
    let file = |edited| User::ApplyFile {
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

fn fleet(dir: &Path, system: &str, user: &str, etc: &[&str]) -> Fleet {
    let type_dir = dir.join("types/workstation");
    fs::create_dir_all(type_dir.join("etc")).unwrap();
    fs::create_dir_all(type_dir.join("users/me")).unwrap();
    for file in etc {
        fs::write(type_dir.join("etc").join(file), "fleet").unwrap();
    }
    fs::write(type_dir.join("system.toml"), system).unwrap();
    fs::write(type_dir.join("users/me/user.toml"), user).unwrap();
    Fleet::new(dir.to_path_buf(), "workstation", &[], &BTreeSet::new()).unwrap()
}

#[test]
fn the_new_record_holds_what_the_fleet_declares_and_keeps_edits_as_edits() {
    let checkout = TempDir::new().unwrap();
    let mut fleet = fleet(
        checkout.path(),
        r#"
packages = ["vim", "zip"]
built_packages = ["mytool"]
services = ["sshd.service"]
"#,
        r#"
shell = "/usr/bin/fish"
groups = ["docker"]
user_services = ["app.service"]
linger = true
"#,
        &["same.conf", "edited.conf", "new.conf"],
    );
    fleet.members[0].user = Some(crate::user::by_uid(0).unwrap());

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

    let after = Record::after(&fleet, &before, machine.dir.path(), &[]).unwrap();
    let names = |set: &BTreeSet<String>| set.iter().cloned().collect::<Vec<_>>();
    assert_eq!(names(&after.packages), ["vim", "zip"]);
    assert_eq!(names(&after.services), ["sshd.service"]);
    let me = &after.users["me"];
    assert_eq!(names(&me.groups), ["docker"]);
    assert_eq!(names(&me.user_services), ["app.service"]);
    assert_eq!(me.shell.as_deref(), Some("/usr/bin/fish"));
    assert!(me.linger);
    let expected = BTreeMap::from([
        (machine.path("same.conf"), md5("fleet")),
        (machine.path("edited.conf"), md5("older")),
    ]);
    assert_eq!(after.etc, expected);
}

#[test]
fn a_user_the_machine_lacked_keeps_their_old_entry() {
    let checkout = TempDir::new().unwrap();
    let fleet = fleet(checkout.path(), "", "linger = true\n", &[]);
    assert!(fleet.members[0].user.is_none());
    let machine = Machine::new();
    let after = Record::after(&fleet, &Record::default(), machine.dir.path(), &[]).unwrap();
    assert!(after.users.is_empty());
}

#[test]
fn what_apply_skipped_keeps_its_old_entry_so_the_next_apply_brings_it() {
    let checkout = TempDir::new().unwrap();
    let mut fleet = fleet(
        checkout.path(),
        "",
        "user_services = [\"app.service\", \"kept.service\"]\nlinger = true\n",
        &[],
    );
    fleet.members[0].user = Some(crate::user::by_uid(0).unwrap());
    let before = record(|r| {
        r.users
            .entry("me".into())
            .or_default()
            .user_services
            .insert("kept.service".into());
    });
    let enable = Service::Unit(Unit::Enable("app.service".into()));
    let disable = Service::Unit(Unit::Disable("dropped.service".into()));
    let skipped = [(0, &enable), (0, &disable), (0, &Service::Linger(true))];
    let machine = Machine::new();
    let after = Record::after(&fleet, &before, machine.dir.path(), &skipped).unwrap();
    let me = &after.users["me"];
    assert_eq!(
        me.user_services,
        BTreeSet::from(["kept.service".to_owned()])
    );
    assert!(!me.linger);
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

#[test]
fn a_revert_in_review_leaves_the_file_as_apply_wrote_it() {
    let machine = Machine::new();
    let path = machine.write("a.conf", "fleet");
    let mut edited = record(|r| {
        r.etc.insert(path.clone(), md5("older"));
    });
    assert!(!changed(&edited, Etc::Write(path.clone())));
    edited.reverted(&Etc::Write(path.clone()).into()).unwrap();
    assert_eq!(edited.etc.get(&path), Some(&md5("fleet")));
    assert!(changed(&edited, Etc::Write(path)));
}

#[test]
fn a_file_review_deleted_or_restored_leaves_the_record() {
    let machine = Machine::new();
    let (a, b, c) = (
        machine.path("unit.d/a.conf"),
        machine.path("unit.d/b.conf"),
        machine.path("c.conf"),
    );
    let mut before = record(|r| {
        for path in [&a, &b, &c] {
            r.etc.insert(path.clone(), md5("x"));
        }
    });
    before
        .reverted(&Etc::Delete(machine.path("unit.d")).into())
        .unwrap();
    assert_eq!(before.etc.keys().collect::<Vec<_>>(), [&c]);
    before
        .reverted(
            &Etc::Restore {
                path: c,
                archive: no_archive(),
            }
            .into(),
        )
        .unwrap();
    assert!(before.etc.is_empty());
}

fn tools(text: &str) -> Tools {
    toml::from_str(text).unwrap()
}

#[test]
fn tools_and_installers_follow_the_same_rule() {
    let before = record(|r| {
        me(r).tools = tools(r#"cargo = ["ripgrep"]"#);
        me(r).installers.insert("bun".into());
    });
    let install = |name: &str| ToolChange::Install {
        manager: Manager::Cargo,
        tool: Tool::Name(name.into()),
    };
    let uninstall = |name: &str| ToolChange::Uninstall {
        manager: Manager::Cargo,
        name: name.into(),
    };
    assert!(changed(&before, install("bat")));
    assert!(!changed(&before, install("ripgrep")));
    assert!(changed(&before, uninstall("ripgrep")));
    assert!(!changed(&before, uninstall("bat")));
    let uv = ToolChange::Install {
        manager: Manager::Uv,
        tool: Tool::Name("ripgrep".into()),
    };
    assert!(changed(&before, uv));

    assert!(changed(&before, User::RunInstaller("rustup".into())));
    assert!(!changed(&before, User::RunInstaller("bun".into())));
}

#[test]
fn installs_a_missing_tool_again_when_the_fleet_changed_its_source() {
    let before = record(|r| {
        me(r).tools =
            tools(r#"cargo = [{ name = "x", git = "https://example.com/x", branch = "a" }]"#);
    });
    let moved = tools(r#"cargo = [{ name = "x", git = "https://example.com/x", branch = "b" }]"#);
    let install = |tools: &Tools| ToolChange::Install {
        manager: Manager::Cargo,
        tool: tools.of(Manager::Cargo)[0].clone(),
    };
    assert!(!changed(&before, install(&before.users["me"].tools)));
    assert!(changed(&before, install(&moved)));
}
