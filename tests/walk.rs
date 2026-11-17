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
    let walk = unknown(root, &HashSet::new(), &HashSet::new(), &skip).unwrap();
    assert_eq!(walk.unknown, [root.join("app")]);
}
