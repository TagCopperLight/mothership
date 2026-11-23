use super::*;

#[test]
fn the_login_range_comes_from_login_defs_or_its_defaults() {
    let defs = "# UID_MIN 5\nUID_MIN\t\t 2000\nUID_MAX 3000\n";
    assert_eq!(login_def(defs, "UID_MIN", 1000).unwrap(), 2000);
    assert_eq!(login_def(defs, "UID_MAX", 60000).unwrap(), 3000);
    assert_eq!(login_def("", "UID_MIN", 1000).unwrap(), 1000);
    assert!(login_def("UID_MIN lots", "UID_MIN", 1000).is_err());
}

#[test]
fn a_nologin_or_false_shell_is_no_person() {
    assert!(can_log_in("/usr/bin/fish"));
    assert!(!can_log_in("/usr/sbin/nologin"));
    assert!(!can_log_in("/bin/false"));
}
