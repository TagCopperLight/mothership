use super::*;

#[test]
fn a_secret_shows_no_contents_but_other_files_do() {
    let dir = tempfile::TempDir::new().unwrap();
    let key = dir.path().join("id");
    fs::write(&key, "-----BEGIN PRIVATE KEY-----").unwrap();
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
