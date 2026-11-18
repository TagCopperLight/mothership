use super::*;

fn owner_only(path: &Path) {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn fleet(dir: &Path, not_secret: &str) -> Fleet {
    let type_dir = dir.join("types/workstation");
    fs::create_dir_all(type_dir.join("etc")).unwrap();
    fs::write(type_dir.join("system.toml"), "").unwrap();
    fs::write(type_dir.join("not-secret.txt"), not_secret).unwrap();
    Fleet::new(dir.to_path_buf(), "workstation", &[], &BTreeSet::new()).unwrap()
}

#[test]
fn a_secret_shows_no_contents_but_other_files_do() {
    let dir = tempfile::TempDir::new().unwrap();
    let fleet = fleet(dir.path(), "");
    let key = dir.path().join("id");
    fs::write(&key, "abc").unwrap();
    owner_only(&key);
    let notes = dir.path().join("notes");
    fs::write(&notes, "hello").unwrap();
    let item = |path: &Path| {
        looks_secret(
            &fleet,
            Item::new(Kind::Home, "x", "").showing(Preview::Contents(path.to_path_buf())),
            path,
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
        .secret(Secret::Guessed);
    assert!(matches!(item.add, Some(Add::Track { encrypt: true, .. })));
}

#[test]
fn not_secret_txt_clears_the_guess() {
    let dir = tempfile::TempDir::new().unwrap();
    let fleet = fleet(dir.path(), "~/.config/zed/*\n");
    let settings = dir.path().join("settings.json");
    fs::write(&settings, "{}").unwrap();
    owner_only(&settings);
    let item = |subject: &str| looks_secret(&fleet, Item::new(Kind::Home, subject, ""), &settings);
    assert_eq!(item("~/.config/zed/settings.json").secret, Secret::No);
    assert_eq!(item("~/.config/gh/hosts.yml").secret, Secret::Guessed);
}
