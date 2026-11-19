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
