use super::*;
use crate::{item::Item, kind::Kind, preview::Preview};

fn owner_only(path: &Path) {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn not_secret(dir: &Path, lines: &str) -> Paths {
    let path = dir.join("not-secret.txt");
    fs::write(&path, lines).unwrap();
    Paths::load(&path).unwrap()
}

#[test]
fn a_secret_shows_no_contents_but_other_files_do() {
    let dir = tempfile::TempDir::new().unwrap();
    let not_secret = not_secret(dir.path(), "");
    let key = dir.path().join("id");
    fs::write(&key, "abc").unwrap();
    owner_only(&key);
    let notes = dir.path().join("notes");
    fs::write(&notes, "hello").unwrap();
    let item = |path: &Path| {
        judge_secret(
            &not_secret,
            Item::new(Kind::Home, Change::Added, "x", "new")
                .showing(Preview::Contents(path.to_path_buf())),
            &Found::at(path, dir.path()),
        )
    };
    let secret = item(&key);
    assert_eq!(secret.secret, Secret::Guessed);
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
    assert!(!Found::at(&app, dir.path()).private());
    owner_only(&app.join("deep/token"));
    assert!(Found::at(&app, dir.path()).private());
}

#[test]
fn a_file_in_a_closed_folder_is_secret() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::TempDir::new().unwrap();
    let token = dir.path().join(".config/app/token");
    fs::create_dir_all(token.parent().unwrap()).unwrap();
    fs::write(&token, "").unwrap();
    assert!(!Found::at(&token, dir.path()).private());
    let app = token.parent().unwrap();
    fs::set_permissions(app, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(Found::at(&token, dir.path()).private());
    assert!(Found::at(&dir.path().join(".config"), dir.path()).private());
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(app, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!Found::at(&token, dir.path()).private());
}

#[test]
fn not_secret_txt_clears_the_guess() {
    let dir = tempfile::TempDir::new().unwrap();
    let not_secret = not_secret(dir.path(), "~/.config/zed/*\n");
    let settings = dir.path().join("settings.json");
    fs::write(&settings, "{}").unwrap();
    owner_only(&settings);
    let item = |subject: &str| {
        judge_secret(
            &not_secret,
            Item::new(Kind::Home, Change::Added, subject, "new"),
            &Found::at(&settings, dir.path()),
        )
    };
    assert_eq!(item("~/.config/zed/settings.json").secret, Secret::No);
    assert_eq!(item("~/.config/gh/hosts.yml").secret, Secret::Guessed);
}
