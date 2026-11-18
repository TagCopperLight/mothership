use super::*;

fn owner_only(path: &Path) {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn a_secret_shows_no_contents_but_other_files_do() {
    let dir = tempfile::TempDir::new().unwrap();
    let key = dir.path().join("id");
    fs::write(&key, "abc").unwrap();
    owner_only(&key);
    let notes = dir.path().join("notes");
    fs::write(&notes, "hello").unwrap();
    let item = |path: &Path| {
        looks_secret(
            Item::new(Kind::Home, "x", "").showing(Preview::Contents(path.to_path_buf())),
            path,
        )
    };
    let secret = item(&key);
    assert!(secret.secret);
    assert!(matches!(secret.preview, Preview::None));
    assert!(matches!(item(&notes).preview, Preview::Contents(_)));
}

#[test]
fn a_folder_is_secret_when_one_of_its_files_is() {
    let dir = tempfile::TempDir::new().unwrap();
    let app = dir.path().join("app");
    fs::create_dir_all(app.join("deep")).unwrap();
    fs::write(app.join("settings"), "").unwrap();
    fs::write(app.join("deep/token"), "").unwrap();
    assert!(!guess_secret(&app));
    owner_only(&app.join("deep/token"));
    assert!(guess_secret(&app));
}

#[test]
fn a_secret_is_added_encrypted() {
    let path = PathBuf::from("/x");
    let item = Item::new(Kind::Home, "x", "")
        .with(Add::Track {
            path,
            encrypt: false,
        })
        .secret();
    assert!(matches!(item.add, Some(Add::Track { encrypt: true, .. })));
}
