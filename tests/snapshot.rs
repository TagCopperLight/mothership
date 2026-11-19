use super::*;
use std::os::unix::fs::symlink;

fn me(home: &Path) -> User {
    let mut user = crate::user::by_uid(unsafe { libc::geteuid() }).unwrap();
    user.home = home.to_path_buf();
    user
}

#[test]
fn a_linked_folder_becomes_a_real_one_holding_what_the_user_reads() {
    let dir = tempfile::TempDir::new().unwrap();
    let home = dir.path().join("home");
    let elsewhere = dir.path().join("elsewhere");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&elsewhere).unwrap();
    fs::write(elsewhere.join("key.txt"), "secret").unwrap();
    symlink(&elsewhere, home.join("c")).unwrap();
    symlink("key.txt", elsewhere.join("link")).unwrap();
    let user = me(&home);

    let file = home.join("c/key.txt");
    let link = home.join("c/link");
    let snapshot = Snapshot::take(&user, &[&file, &link]).unwrap();
    let copied = snapshot.path(&user, &file).unwrap();
    assert!(
        !fs::symlink_metadata(copied.parent().unwrap())
            .unwrap()
            .is_symlink()
    );
    assert_eq!(fs::read_to_string(&copied).unwrap(), "secret");
    let copied_link = snapshot.path(&user, &link).unwrap();
    assert_eq!(fs::read_link(copied_link).unwrap(), Path::new("key.txt"));

    let again = Snapshot::take(&user, &[&file, &link]).unwrap();
    assert_eq!(snapshot.digest(), again.digest());
    fs::write(elsewhere.join("key.txt"), "changed").unwrap();
    let changed = Snapshot::take(&user, &[&file, &link]).unwrap();
    assert_ne!(snapshot.digest(), changed.digest());
}

#[test]
fn special_files_are_left_out_and_paths_stay_in_the_home() {
    let dir = tempfile::TempDir::new().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(home.join("d")).unwrap();
    let fifo = std::ffi::CString::new(home.join("d/fifo").to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    fs::write(home.join("d/file"), "x").unwrap();
    let user = me(&home);

    let snapshot = Snapshot::take(&user, &[&home.join("d")]).unwrap();
    let copied = snapshot.path(&user, &home.join("d")).unwrap();
    assert!(copied.join("file").exists());
    assert!(!copied.join("fifo").exists());

    assert!(Snapshot::take(&user, &[&home.join("../escape")]).is_err());
    assert!(Snapshot::take(&user, &[Path::new("/etc/passwd")]).is_err());
}

#[test]
fn the_digest_tells_apart_names_that_print_the_same() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
    let digest = |name: &[u8]| {
        let dir = tempfile::TempDir::new().unwrap();
        let home = dir.path().join("home");
        fs::create_dir_all(home.join("app")).unwrap();
        fs::write(home.join("app").join(OsStr::from_bytes(name)), "x").unwrap();
        let user = me(&home);
        Snapshot::take(&user, &[&home.join("app")])
            .unwrap()
            .digest()
            .to_owned()
    };
    assert_ne!(digest(b"caf\xe9"), digest(b"caf\xe8"));
    assert_ne!(digest(b"a"), digest(b"a\nf 644 b"));
}

#[test]
fn a_stream_cannot_make_root_write_through_a_link_it_created() {
    let dir = tempfile::TempDir::new().unwrap();
    let home = dir.path().join("home");
    let outside = dir.path().join("outside");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let write_all = |entries: Vec<Entry>| {
        let mut dirs = HashSet::new();
        let mut fields = Fields::new();
        entries
            .into_iter()
            .try_for_each(|entry| write(&home, entry, &mut dirs, &mut fields))
    };
    let link = || Entry::Link {
        rel: "a".into(),
        target: outside.clone(),
    };
    let file = || Entry::File {
        rel: "a/passwd".into(),
        mode: 0o644,
        file: File::open("/dev/null").unwrap(),
    };
    assert!(write_all(vec![link(), file()]).is_err());
    assert!(!outside.join("passwd").exists());
    let dir_entry = || Entry::Dir {
        rel: "a".into(),
        mode: 0o755,
    };
    assert!(write_all(vec![dir_entry()]).is_err());
}

#[test]
fn copied_modes_drop_setuid_and_a_path_written_twice_fails() {
    let dir = tempfile::TempDir::new().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("f"), "x").unwrap();
    fs::set_permissions(home.join("f"), fs::Permissions::from_mode(0o4755)).unwrap();
    let user = me(&home);
    let snapshot = Snapshot::take(&user, &[&home.join("f")]).unwrap();
    let mode = fs::metadata(snapshot.path(&user, &home.join("f")).unwrap())
        .unwrap()
        .mode();
    assert_eq!(mode & 0o7777, 0o755);

    let mut dirs = HashSet::new();
    let mut fields = Fields::new();
    let entry = || Entry::Link {
        rel: "l".into(),
        target: "x".into(),
    };
    write(dir.path(), entry(), &mut dirs, &mut fields).unwrap();
    assert!(write(dir.path(), entry(), &mut dirs, &mut fields).is_err());
}
