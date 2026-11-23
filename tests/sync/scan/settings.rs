use super::*;
use std::fs;

#[test]
fn a_browser_that_never_started_has_no_prefs_to_report() {
    let dir = tempfile::tempdir().unwrap();
    let user_js = dir.path().join("user.js");
    fs::write(&user_js, "user_pref(\"a\", 1);\n").unwrap();
    let me = crate::support::current_user();
    assert!(prefs(&me, &user_js).unwrap().is_empty());
}
