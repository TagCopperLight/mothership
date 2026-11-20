use super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn recipients_are_named_by_the_comment_above_them() {
    let text = "# laptop\nage1aaa\n\n# recovery\nage1bbb\n";
    let list = parse(text).unwrap();
    assert_eq!(
        list,
        [
            Recipient {
                name: "laptop".into(),
                key: "age1aaa".into()
            },
            Recipient {
                name: "recovery".into(),
                key: "age1bbb".into()
            },
        ]
    );
    assert_eq!(format(&list), text);
}

#[test]
fn a_key_without_a_name_is_refused() {
    assert!(parse("# laptop\nage1aaa\nage1bbb\n").is_err());
}

#[test]
fn a_list_names_each_key_once() {
    assert!(parse("# laptop\n# desktop\nage1aaa\n").is_err());
    assert!(parse("# laptop\nage1aaa\n# trailing\n").is_err());
    assert!(parse("# laptop\nage1aaa\n# laptop\nage1bbb\n").is_err());
    assert!(parse("# laptop\nage1aaa\n# desktop\nage1aaa\n").is_err());
}

#[test]
fn opens_what_it_encrypts_for_its_own_key_only() {
    let dir = tempfile::TempDir::new().unwrap();
    let (mine, other) = (dir.path().join("mine.txt"), dir.path().join("other.txt"));
    let (mine, other) = (make_key(&mine).unwrap(), make_key(&other).unwrap());
    let list = dir.path().join(RECIPIENTS);
    fs::write(&list, format!("# mine\n{}\n", public(&mine))).unwrap();
    assert!(holds(&list, &public(&mine)).unwrap());
    assert!(!holds(&list, &public(&other)).unwrap());

    let sealed = dir.path().join("secret.age");
    fs::write(&sealed, crate::support::seal(&list, b"psk=1")).unwrap();
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
