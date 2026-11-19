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
