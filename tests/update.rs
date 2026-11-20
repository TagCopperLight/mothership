use super::*;

#[test]
fn reads_the_pin_and_ignores_fields_it_doesnt_know() {
    let dir = tempfile::TempDir::new().unwrap();
    assert!(pin(dir.path()).unwrap().is_none());
    fs::write(
        dir.path().join(PIN),
        "version = \"0.2.0\"\nlater = 1\n[sha256]\nx86_64 = \"abc\"\nmips = \"def\"\n",
    )
    .unwrap();
    let pin = pin(dir.path()).unwrap().unwrap();
    assert_eq!(pin.version, "0.2.0");
    assert_eq!(pin.sha256["x86_64"], "abc");
}
