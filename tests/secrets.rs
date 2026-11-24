use super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn opens_what_it_encrypts_for_its_own_key_only() {
    let dir = tempfile::TempDir::new().unwrap();
    let (mine, other) = (dir.path().join("mine.txt"), dir.path().join("other.txt"));
    let (mine, other) = (make_key(&mine).unwrap(), make_key(&other).unwrap());
    let list = [Recipient {
        name: "mine".into(),
        key: public(&mine),
    }];
    let sealed = dir.path().join("secret.age");
    fs::write(&sealed, encrypt_for(&list, b"psk=1").unwrap()).unwrap();
    assert!(
        fs::read_to_string(&sealed)
            .unwrap()
            .starts_with("-----BEGIN AGE")
    );
    let keys = |key: &x25519::Identity| {
        let mut keys = Keys::default();
        keys.push(key.clone());
        keys
    };
    assert_eq!(keys(&mine).decrypt(&sealed).unwrap(), b"psk=1");
    assert!(keys(&other).decrypt(&sealed).is_err());
}

#[test]
fn a_key_file_is_for_its_owner_only() {
    let dir = tempfile::TempDir::new().unwrap();
    let key = dir.path().join("deep/key.txt");
    let made = make_key(&key).unwrap();
    assert_eq!(
        fs::metadata(&key).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        public(&parse_key(&fs::read(&key).unwrap(), &key).unwrap()),
        public(&made)
    );
    assert!(make_key(&key).is_err());
}
