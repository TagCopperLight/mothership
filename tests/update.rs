use super::*;

#[test]
fn reads_the_pin_and_ignores_fields_it_doesnt_know() {
    let dir = tempfile::TempDir::new().unwrap();
    assert!(Manifest::read(dir.path()).unwrap().pin().is_none());
    fs::write(
        dir.path().join(MANIFEST),
        "version = \"0.2.0\"\nlater = 1\n[sha256]\nx86_64 = \"abc\"\nmips = \"def\"\n",
    )
    .unwrap();
    let manifest = Manifest::read(dir.path()).unwrap();
    let pin = manifest.pin().unwrap();
    assert_eq!(pin.version, "0.2.0");
    assert_eq!(pin.sha256["x86_64"], "abc");
}
