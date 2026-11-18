use super::*;

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
        looks_secret(
            &not_secret,
            Item::new(Kind::Home, Change::Added, "x", "new")
                .showing(Preview::Contents(path.to_path_buf())),
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
    let path = std::path::PathBuf::from("/x");
    let item = Item::new(Kind::Home, Change::Added, "x", "new")
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
    let not_secret = not_secret(dir.path(), "~/.config/zed/*\n");
    let settings = dir.path().join("settings.json");
    fs::write(&settings, "{}").unwrap();
    owner_only(&settings);
    let item = |subject: &str| {
        looks_secret(
            &not_secret,
            Item::new(Kind::Home, Change::Added, subject, "new"),
            &settings,
        )
    };
    assert_eq!(item("~/.config/zed/settings.json").secret, Secret::No);
    assert_eq!(item("~/.config/gh/hosts.yml").secret, Secret::Guessed);
}

fn in_section(mut item: Item, section: Section) -> Item {
    item.section = section;
    item
}

#[test]
fn the_sign_shows_what_the_machine_gained_or_what_apply_brings_it() {
    let installed = Item::new(Kind::Package, Change::Added, "htop", "installed")
        .reverting(Revert::Uninstall("htop".into()));
    assert_eq!(installed.sign().0, "+");
    assert_eq!(installed.verb(), "installed");
    let dropped = in_section(installed, Section::Fleet);
    assert_eq!(dropped.sign().0, "-");
    assert_eq!(dropped.verb(), "uninstall");
}

#[test]
fn the_fleet_writing_a_file_says_whether_it_creates_changes_or_deletes_it() {
    let file = |change| {
        let revert = Revert::ApplyFile {
            path: "/home/me/x".into(),
            edited: false,
        };
        let item = Item::new(Kind::File, change, "~/x", "").reverting(revert);
        in_section(item, Section::Fleet)
    };
    assert_eq!(file(Change::Removed).verb(), "create");
    assert_eq!(file(Change::Edited).verb(), "update");
    assert_eq!(file(Change::Added).verb(), "delete");
    assert_eq!(file(Change::Added).sign().0, "-");
}

#[test]
fn problems_need_attention_and_changes_start_on_the_machine() {
    let section = |change| Item::new(Kind::Home, change, "x", "").section;
    assert_eq!(section(Change::InClear), Section::Attention);
    assert_eq!(section(Change::Unreadable), Section::Attention);
    assert_eq!(section(Change::Edited), Section::Machine);
}
