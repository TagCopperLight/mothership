use super::*;
use crate::secrets::{self, Recipient};
use std::fs;

#[test]
fn a_folder_shows_every_file_whole() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::create_dir(dir.path().join("sub")).unwrap();
    fs::write(dir.path().join("a"), "one\ntwo\n").unwrap();
    let long = format!("{}\nhidden", " ".repeat(500));
    fs::write(dir.path().join("sub/b"), &long).unwrap();
    let lines = contents(dir.path()).unwrap();
    assert_eq!(lines[..3], ["── a", "one", "two"]);
    assert_eq!(lines[3], "── sub/b");
    assert_eq!(lines.len(), 6);
    assert_eq!(lines[5], "hidden");
}

#[test]
fn a_long_file_is_not_cut() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let text: String = (0..100).map(|n| format!("line {n}\n")).collect();
    fs::write(file.path(), &text).unwrap();
    assert_eq!(contents(file.path()).unwrap().len(), 100);
}

#[test]
fn a_diff_shows_every_changed_line() {
    let dir = tempfile::TempDir::new().unwrap();
    let (old, new) = (dir.path().join("old"), dir.path().join("new"));
    let before: String = (0..200).map(|n| format!("{n}\n")).collect();
    fs::write(&old, &before).unwrap();
    fs::write(&new, before.replace("150\n", "changed\n")).unwrap();
    let lines = diff(&old, &new).unwrap();
    assert!(lines[0].starts_with("@@"));
    assert!(lines.contains(&"-150".to_owned()));
    assert!(lines.contains(&"+changed".to_owned()));
}

#[test]
fn a_secret_shows_what_the_fleet_holds_decrypted() {
    let dir = tempfile::TempDir::new().unwrap();
    let key = secrets::make_key(&dir.path().join("key")).unwrap();
    let mut keys = Keys::default();
    keys.push(key.clone());
    let list = [Recipient {
        name: "me".into(),
        key: secrets::public(&key),
    }];
    let source = dir.path().join("wifi.conf.age");
    fs::write(&source, secrets::encrypt_for(&list, b"psk=abc\n").unwrap()).unwrap();
    let now = dir.path().join("wifi.conf");
    fs::write(&now, "psk=xyz\n").unwrap();
    let alone = Preview::Decrypted {
        source: source.clone(),
        keys: keys.clone(),
        now: None,
    };
    assert_eq!(alone.render(&Seen::default()).unwrap(), ["psk=abc"]);
    let against = Preview::Decrypted {
        source,
        keys,
        now: Some(now),
    };
    let lines = against.render(&Seen::default()).unwrap();
    assert!(lines.contains(&"-psk=abc".to_owned()));
    assert!(lines.contains(&"+psk=xyz".to_owned()));
}

#[test]
fn a_user_unit_is_read_from_its_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("app.service");
    fs::write(&path, "[Service]\nExecStart=/bin/true\n").unwrap();
    let user = crate::support::current_user();
    let shown = Preview::AsUser { path, user };
    assert_eq!(
        shown.render(&Seen::default()).unwrap(),
        ["[Service]", "ExecStart=/bin/true"]
    );
}

#[test]
fn what_was_shown_must_match_what_is_added() {
    let file = tempfile::NamedTempFile::new().unwrap();
    fs::write(file.path(), "one").unwrap();
    let digest = |path: &Path| hash::sha256(&file::content(path).unwrap());
    let seen = Seen::default();
    assert!(seen.check(&digest(file.path())).is_err());
    Preview::Contents(file.path().to_path_buf()).show(&seen, false);
    seen.check(&digest(file.path())).unwrap();
    fs::write(file.path(), "two").unwrap();
    assert!(seen.check(&digest(file.path())).is_err());
}
