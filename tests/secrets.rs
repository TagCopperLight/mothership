use super::*;

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
fn opens_what_it_encrypts_for_its_own_key_only() {
    let dir = tempfile::TempDir::new().unwrap();
    let (mine, other) = (dir.path().join("mine.txt"), dir.path().join("other.txt"));
    make_key(&mine, None).unwrap();
    make_key(&other, None).unwrap();
    let public = |key: &Path| read_key(key).unwrap().unwrap().to_public().to_string();
    let list = dir.path().join(RECIPIENTS);
    fs::write(&list, format!("# mine\n{}\n", public(&mine))).unwrap();
    assert!(opens(&list, &mine).unwrap());
    assert!(!opens(&list, &other).unwrap());
    assert!(!opens(&list, &dir.path().join("missing.txt")).unwrap());

    let sealed = dir.path().join("secret.age");
    fs::write(&sealed, encrypt(&list, b"psk=1").unwrap()).unwrap();
    assert!(
        fs::read_to_string(&sealed)
            .unwrap()
            .starts_with("-----BEGIN AGE")
    );
    let keys = |path: &Path| Keys::load(std::iter::once(path.to_path_buf())).unwrap();
    assert_eq!(keys(&mine).decrypt(&sealed).unwrap(), b"psk=1");
    assert!(keys(&other).decrypt(&sealed).is_err());
}

#[test]
fn a_key_file_is_for_its_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::TempDir::new().unwrap();
    let key = dir.path().join("deep/key.txt");
    make_key(&key, None).unwrap();
    assert_eq!(
        fs::metadata(&key).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(make_key(&key, None).is_err());
}
