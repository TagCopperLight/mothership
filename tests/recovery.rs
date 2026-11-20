use super::*;
use std::fs;

#[test]
fn a_typed_key_goes_into_the_manifest_next_to_the_pin() {
    let dir = tempfile::TempDir::new().unwrap();
    let key = x25519::Identity::generate();
    fs::write(dir.path().join(MANIFEST), "version = \"0.2.0\"\n").unwrap();
    let typed: &'static str = Box::leak(format!("t\n{}\n", key.to_string().expose_secret()).into());
    run(dir.path(), &mut Prompt::answering(typed)).unwrap();
    let manifest = Manifest::read(dir.path()).unwrap();
    assert_eq!(manifest.recovery().unwrap().key, secrets::public(&key));
    assert_eq!(manifest.pin().unwrap().version, "0.2.0");
    let again = run(dir.path(), &mut Prompt::answering("n\n")).unwrap_err();
    assert_eq!(again.to_string(), "the fleet has a recovery key already");
}

#[test]
fn enter_makes_a_new_key() {
    let dir = tempfile::TempDir::new().unwrap();
    run(dir.path(), &mut Prompt::answering("n\n")).unwrap();
    let public = Manifest::read(dir.path()).unwrap().recovery().unwrap().key;
    assert!(public.starts_with("age1"));
}
