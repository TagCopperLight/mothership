use super::*;

#[test]
fn chezmoi_lines_keep_names_that_are_not_utf8() {
    let out = b"/home/me/.bashrc\n/home/me/caf\xe9\n";
    let paths: Vec<&Path> = lines(out).collect();
    assert_eq!(paths[0], Path::new("/home/me/.bashrc"));
    assert_eq!(
        paths[1].as_os_str().as_bytes(),
        b"/home/me/caf\xe9".as_slice()
    );
}

#[test]
fn the_config_text_stays_byte_for_byte_the_same() {
    let config = ChezmoiConfig {
        source: PathBuf::from("/fleet/types/workstation/users/me/home"),
        identity: PathBuf::from("/home/me/.config/mothership/key.txt"),
        recipients: PathBuf::from("/home/me/.config/mothership/recipients.txt"),
        profile: "workstation".into(),
        user: "me".into(),
        logins: None,
    };
    let text = config_text(&config).unwrap();
    let expected = r#"encryption = "age"
sourceDir = "/fleet/types/workstation/users/me/home"

[age]
identity = "/home/me/.config/mothership/key.txt"
recipientsFile = "/home/me/.config/mothership/recipients.txt"
useBuiltin = true

[data]
profile = "workstation"
user = "me"
"#;
    assert_eq!(text, expected);
}

#[test]
fn a_source_name_decodes_to_its_target() {
    assert_eq!(
        target_of(Path::new(
            "private_dot_ssh/encrypted_private_id_ed25519.age"
        )),
        (PathBuf::from(".ssh/id_ed25519"), true)
    );
    assert_eq!(
        target_of(Path::new("dot_config/app/settings.json.tmpl")),
        (PathBuf::from(".config/app/settings.json"), false)
    );
}

#[test]
fn the_login_keys_reach_chezmoi_only_when_ssh_from_lists_some() {
    let config = |logins| ChezmoiConfig {
        source: PathBuf::from("/home"),
        identity: PathBuf::from("/key.txt"),
        recipients: PathBuf::from("/recipients.txt"),
        profile: "server".into(),
        user: "me".into(),
        logins,
    };
    let text = config_text(&config(Some(vec!["ssh-ed25519 AAAA laptop/me".into()]))).unwrap();
    assert!(text.contains(r#"authorized_keys = ["ssh-ed25519 AAAA laptop/me"]"#));
    assert!(
        !config_text(&config(None))
            .unwrap()
            .contains("authorized_keys")
    );
}
