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
        source: PathBuf::from("/fleet/homes/me"),
        identity: PathBuf::from("/home/me/.config/mothership/key.txt"),
        recipients: PathBuf::from("/fleet/types/workstation/users/me/recipients.txt"),
        profile: "workstation".into(),
        user: "me".into(),
        fleet: PathBuf::from("/fleet"),
    };
    let text = config_text(&config).unwrap();
    let expected = r#"encryption = "age"
sourceDir = "/fleet/homes/me"

[age]
identity = "/home/me/.config/mothership/key.txt"
recipientsFile = "/fleet/types/workstation/users/me/recipients.txt"
useBuiltin = true

[data]
fleet = "/fleet"
profile = "workstation"
user = "me"
"#;
    assert_eq!(text, expected);
}
