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

#[test]
fn a_shared_home_file_is_for_every_list_of_every_user_linking_it() {
    let dir = tempfile::TempDir::new().unwrap();
    let fleet = dir.path();
    for (type_name, user, text) in [
        (
            "workstation",
            "tag",
            "# laptop\nage1tag\n\n# recovery\nage1rec\n",
        ),
        (
            "server",
            "dev",
            "# minipc\nage1dev\n\n# recovery\nage1rec\n",
        ),
    ] {
        let users = fleet.join("types").join(type_name).join("users").join(user);
        fs::create_dir_all(&users).unwrap();
        fs::write(users.join(RECIPIENTS), text).unwrap();
        fs::create_dir_all(fleet.join("homes").join(user)).unwrap();
        std::os::unix::fs::symlink(
            "../../shared/dot_token",
            fleet.join("homes").join(user).join("dot_token"),
        )
        .unwrap();
    }
    fs::create_dir_all(fleet.join("shared")).unwrap();
    fs::write(fleet.join("shared/dot_token"), "").unwrap();
    let keys = |list: Vec<Recipient>| list.into_iter().map(|r| r.key).collect::<Vec<_>>();
    let link = fleet.join("homes/dev/dot_token");
    assert_eq!(
        keys(home_recipients(fleet, &link, None).unwrap()),
        ["age1dev", "age1rec", "age1tag"]
    );
    let dev_list = fleet.join("types/server/users/dev").join(RECIPIENTS);
    let new = [Recipient {
        name: "minipc".into(),
        key: "age1new".into(),
    }];
    assert_eq!(
        keys(home_recipients(fleet, &link, Some((&dev_list, &new))).unwrap()),
        ["age1new", "age1tag", "age1rec"]
    );
}
