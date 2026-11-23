use super::*;

#[test]
fn writes_prefs_into_user_js() {
    let dir = tempfile::TempDir::new().unwrap();
    let user_js = dir.path().join("user.js");
    fs::write(&user_js, "user_pref(\"a\",1);\nuser_pref(\"ab\", 2);\n").unwrap();
    set_pref(&user_js, "a", "3").unwrap();
    set_pref(&user_js, "c", "\"text\"").unwrap();
    assert_eq!(
        fs::read_to_string(&user_js).unwrap(),
        "user_pref(\"a\", 3);\nuser_pref(\"ab\", 2);\nuser_pref(\"c\", \"text\");\n"
    );
}

#[test]
fn encrypting_keeps_a_group_readable_file_private_and_follows_no_link() {
    let dir = tempfile::TempDir::new().unwrap();
    let outside = dir.path().join("outside");
    fs::write(&outside, "x").unwrap();
    fs::set_permissions(&outside, fs::Permissions::from_mode(0o644)).unwrap();
    let folder = dir.path().join("folder");
    fs::create_dir(&folder).unwrap();
    let file = folder.join("token");
    fs::write(&file, "x").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
    std::os::unix::fs::symlink(&outside, folder.join("link")).unwrap();
    keep_owner_only(&folder).unwrap();
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&file), 0o600);
    assert_eq!(mode(&outside), 0o644);
}
