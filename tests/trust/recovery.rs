use super::*;
use std::fs;

#[test]
fn a_claimed_key_goes_into_the_manifest_next_to_the_pin() {
    let dir = tempfile::TempDir::new().unwrap();
    let key = x25519::Identity::generate();
    fs::write(dir.path().join(MANIFEST), "version = \"0.2.0\"\n").unwrap();
    claim(dir.path(), &key).unwrap();
    let manifest = Manifest::read(dir.path()).unwrap();
    assert_eq!(manifest.recovery().unwrap().key, secrets::public(&key));
    assert_eq!(manifest.pin().unwrap().version, "0.2.0");
}

#[test]
fn claim_keeps_the_named_key_and_refuses_another() {
    let dir = tempfile::TempDir::new().unwrap();
    let key = x25519::Identity::generate();
    claim(dir.path(), &key).unwrap();
    claim(dir.path(), &key).unwrap();
    let other = x25519::Identity::generate();
    let refused = claim(dir.path(), &other).unwrap_err();
    assert_eq!(refused.to_string(), wrong_key().to_string());
    let named = Manifest::read(dir.path()).unwrap().recovery().unwrap().key;
    assert_eq!(named, secrets::public(&key));
}

#[test]
fn n_makes_a_new_key() {
    let key = new_key(&mut Prompt::answering("n\n")).unwrap();
    assert!(secrets::public(&key).starts_with("age1"));
}
