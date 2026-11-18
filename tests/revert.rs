use super::*;
use tempfile::TempDir;

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn writes_with_the_mode_into_new_folders_and_leaves_no_temporary_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("sudoers.d/wheel");
    write(&path, b"%wheel ALL=(ALL:ALL) ALL\n", Mode(0o440)).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"%wheel ALL=(ALL:ALL) ALL\n");
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode, 0o440);
    assert_eq!(names(&dir.path().join("sudoers.d")), ["wheel"]);
}

#[test]
fn writing_replaces_a_file_and_a_link_without_following_it() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("a.conf");
    fs::write(&file, "old").unwrap();
    write(&file, b"new", Mode(0o644)).unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "new");

    let target = dir.path().join("target");
    fs::write(&target, "untouched").unwrap();
    let link_path = dir.path().join("link.conf");
    unix::fs::symlink(&target, &link_path).unwrap();
    write(&link_path, b"file now", Mode(0o644)).unwrap();
    assert!(!fs::symlink_metadata(&link_path).unwrap().is_symlink());
    assert_eq!(fs::read_to_string(&target).unwrap(), "untouched");
}

#[test]
fn links_over_a_file_and_leaves_no_temporary_link() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("resolv.conf");
    fs::write(&path, "nameserver 1.1.1.1\n").unwrap();
    link(&path, Path::new("/run/systemd/resolve/stub-resolv.conf")).unwrap();
    assert_eq!(
        fs::read_link(&path).unwrap(),
        Path::new("/run/systemd/resolve/stub-resolv.conf")
    );
    link(&path, Path::new("/run/other")).unwrap();
    assert_eq!(fs::read_link(&path).unwrap(), Path::new("/run/other"));
    assert_eq!(names(dir.path()), ["resolv.conf"]);
}
