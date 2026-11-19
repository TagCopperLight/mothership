use super::*;

#[test]
fn a_folder_of_skipped_paths_and_empty_folders_is_not_reported() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path();
    fs::create_dir_all(root.join("links/a.wants")).unwrap();
    fs::write(root.join("links/a.wants/b"), "").unwrap();
    fs::create_dir_all(root.join("links/empty")).unwrap();
    fs::create_dir_all(root.join("app")).unwrap();
    fs::write(root.join("app/settings"), "").unwrap();
    let skip = |path: &Path| path.ends_with("a.wants");
    let walk = unknown(root, &HashSet::new(), &HashSet::new(), &skip, &|_| false).unwrap();
    assert_eq!(walk.unknown, [root.join("app")]);
}

#[test]
fn a_new_folder_opens_when_an_ignore_line_names_something_inside() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path();
    fs::create_dir_all(root.join("share/docker/images")).unwrap();
    fs::write(root.join("share/docker/images/a"), "").unwrap();
    fs::create_dir_all(root.join("share/app")).unwrap();
    fs::write(root.join("share/app/settings"), "").unwrap();
    fs::create_dir_all(root.join("other")).unwrap();
    fs::write(root.join("other/settings"), "").unwrap();
    let ignored = |path: &Path| path.ends_with("share/docker");
    let walk = unknown(root, &HashSet::new(), &HashSet::new(), &|_| false, &ignored).unwrap();
    let mut found = walk.unknown.clone();
    found.sort();
    assert_eq!(
        found,
        [
            root.join("other"),
            root.join("share/app"),
            root.join("share/docker")
        ]
    );
}

#[test]
fn writes_with_the_mode_into_new_folders_and_leaves_no_temporary_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("sudoers.d/wheel");
    write(&path, b"%wheel ALL=(ALL:ALL) ALL\n", 0o440).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"%wheel ALL=(ALL:ALL) ALL\n");
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode, 0o440);
    assert_eq!(
        fs::read_dir(dir.path().join("sudoers.d")).unwrap().count(),
        1
    );
}

#[test]
fn writing_replaces_a_file_and_a_link_without_following_it() {
    let dir = tempfile::TempDir::new().unwrap();
    let file = dir.path().join("a.conf");
    fs::write(&file, "old").unwrap();
    write(&file, b"new", 0o644).unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "new");

    let target = dir.path().join("target");
    fs::write(&target, "untouched").unwrap();
    let link_path = dir.path().join("link.conf");
    std::os::unix::fs::symlink(&target, &link_path).unwrap();
    write(&link_path, b"file now", 0o644).unwrap();
    assert!(!fs::symlink_metadata(&link_path).unwrap().is_symlink());
    assert_eq!(fs::read_to_string(&target).unwrap(), "untouched");
}
