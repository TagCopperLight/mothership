use super::*;

#[test]
fn reads_the_pin_and_ignores_fields_it_doesnt_know() {
    let dir = tempfile::TempDir::new().unwrap();
    assert!(pin(dir.path()).unwrap().is_none());
    fs::write(
        dir.path().join(PIN),
        "version = \"0.2.0\"\nsha256 = \"abc\"\nlater = 1\n",
    )
    .unwrap();
    let pin = pin(dir.path()).unwrap().unwrap();
    assert_eq!(
        (pin.version.as_str(), pin.sha256.as_str()),
        ("0.2.0", "abc")
    );
}
