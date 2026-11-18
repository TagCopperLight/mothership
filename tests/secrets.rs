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
fn chezmoi_must_encrypt_for_the_types_recipients() {
    let type_dir = Path::new("/fleet/types/workstation");
    let config = |recipients: &str| {
        serde_json::json!({
            "encryption": "age",
            "age": { "identity": "/nowhere/key.txt", "recipientsFile": recipients },
        })
    };
    let secrets = Secrets::new(&config("/fleet/types/workstation/recipients.txt"), type_dir);
    assert!(!secrets.unwrap().open);
    assert!(Secrets::new(&config("/fleet/types/server/recipients.txt"), type_dir).is_err());
}
