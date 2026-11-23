use super::*;
use crate::{sync::item::Item, sync::kind::Kind, sync::preview::Preview};

fn owner_only(path: &Path) {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn a_secret_keeps_its_preview() {
    let dir = tempfile::TempDir::new().unwrap();
    let key = dir.path().join("id");
    fs::write(&key, "abc").unwrap();
    owner_only(&key);
    let notes = dir.path().join("notes");
    fs::write(&notes, "hello").unwrap();
    let item = |path: &Path| {
        judge_secret(
            Item::new(Kind::Home, Change::Added, "x", "new")
                .showing(Preview::Contents(path.to_path_buf())),
            &Found::at(path, Some(dir.path())),
            true,
        )
    };
    let secret = item(&key);
    assert_eq!(secret.secret, Secret::Guessed);
    assert!(matches!(secret.preview, Preview::Contents(_)));
    assert!(matches!(item(&notes).preview, Preview::Contents(_)));
}

#[test]
fn a_folder_is_secret_when_one_of_its_files_is() {
    let dir = tempfile::TempDir::new().unwrap();
    let app = dir.path().join("app");
    fs::create_dir_all(app.join("deep")).unwrap();
    fs::write(app.join("settings"), "").unwrap();
    fs::write(app.join("deep/token"), "").unwrap();
    assert!(!Found::at(&app, Some(dir.path())).private());
    owner_only(&app.join("deep/token"));
    assert!(Found::at(&app, Some(dir.path())).private());
}

#[test]
fn a_file_in_a_closed_folder_is_secret() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::TempDir::new().unwrap();
    let token = dir.path().join(".config/app/token");
    fs::create_dir_all(token.parent().unwrap()).unwrap();
    fs::write(&token, "").unwrap();
    assert!(!Found::at(&token, Some(dir.path())).private());
    let app = token.parent().unwrap();
    fs::set_permissions(app, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(Found::at(&token, Some(dir.path())).private());
    assert!(Found::at(&dir.path().join(".config"), Some(dir.path())).private());
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(app, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!Found::at(&token, Some(dir.path())).private());
}

#[test]
fn a_tracked_file_counts_only_its_own_rights() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::TempDir::new().unwrap();
    let font = dir.path().join(".local/share/fonts/a.otf");
    fs::create_dir_all(font.parent().unwrap()).unwrap();
    fs::write(&font, "").unwrap();
    let share = dir.path().join(".local/share");
    fs::set_permissions(&share, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(Found::at(&font, Some(dir.path())).private());
    assert!(!Found::at(&font, None).private());
    owner_only(&font);
    assert!(Found::at(&font, None).private());
}

#[test]
fn only_a_new_file_is_guessed_from_its_rights() {
    let dir = tempfile::TempDir::new().unwrap();
    let settings = dir.path().join("settings.json");
    fs::write(&settings, "{}").unwrap();
    owner_only(&settings);
    let item = |new| {
        let item = Item::new(Kind::Home, Change::Edited, "~/settings.json", "");
        judge_secret(item, &Found::at(&settings, Some(dir.path())), new).secret
    };
    assert_eq!(item(true), Secret::Guessed);
    assert_eq!(item(false), Secret::No);
}

#[test]
fn a_token_makes_even_a_tracked_file_secret() {
    let dir = tempfile::TempDir::new().unwrap();
    let hosts = dir.path().join("hosts.yml");
    fs::write(&hosts, format!("oauth_token: gho_{}\n", "a".repeat(36))).unwrap();
    let item = Item::new(Kind::Home, Change::Edited, "~/hosts.yml", "");
    let item = judge_secret(item, &Found::at(&hosts, None), false);
    assert_eq!(item.secret, Secret::Token);
    assert_eq!(item.note, "holds what looks like a GitHub token");
}

#[test]
fn a_token_needs_its_prefix_to_start_a_word_and_enough_characters() {
    let long = "a".repeat(36);
    assert_eq!(token_in(&format!("x ghp_{long}")), Some("a GitHub token"));
    assert_eq!(token_in(&format!("xghp_{long}")), None);
    assert_eq!(token_in("ghp_short"), None);
    assert_eq!(token_in("risk-management-for-everyone"), None);
    assert_eq!(
        token_in("-----BEGIN OPENSSH PRIVATE KEY-----"),
        Some("a private key")
    );
}
