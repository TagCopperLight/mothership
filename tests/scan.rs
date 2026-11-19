use super::*;
use crate::secrets::Storage;

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
fn a_fleet_secret_is_not_guessed_again() {
    let item = Item::new(Kind::Etc, Change::Edited, "/etc/x", "edited")
        .guessing(Path::new("/etc/x"))
        .secret(Secret::Fleet);
    assert!(item.guess.is_none());
    assert_eq!(item.secret, Secret::Fleet);
}

#[test]
fn a_secret_is_added_encrypted() {
    let path = std::path::PathBuf::from("/x");
    let item = Item::new(Kind::Home, Change::Added, "x", "new")
        .with(Add::Track {
            path,
            storage: Storage::Clear,
        })
        .secret(Secret::Guessed);
    assert!(matches!(
        item.add,
        Adding::Offered(Add::Track {
            storage: Storage::Encrypted,
            ..
        })
    ));
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

fn in_section(mut item: Item, section: Section) -> Item {
    item.section = section;
    item
}

#[test]
fn the_sign_shows_what_the_machine_gained_or_what_apply_brings_it() {
    let installed = Item::new(Kind::Package, Change::Added, "htop", "installed")
        .reverting(revert::Machine::Uninstall("htop".into()));
    assert_eq!(installed.sign().0, "+");
    assert_eq!(installed.verb(), "installed");
    let dropped = in_section(installed, Section::Fleet);
    assert_eq!(dropped.sign().0, "-");
    assert_eq!(dropped.verb(), "uninstall");
}

#[test]
fn the_fleet_writing_a_file_says_whether_it_creates_changes_or_deletes_it() {
    let file = |change| {
        let revert = revert::User::ApplyFile {
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

#[test]
fn items_sort_by_section_area_change_then_subject() {
    let dir = tempfile::TempDir::new().unwrap();
    let type_dir = dir.path().join("types/workstation");
    fs::create_dir_all(&type_dir).unwrap();
    fs::write(type_dir.join(crate::fleet::SYSTEM_TOML), "").unwrap();
    let fleet = Fleet::new(dir.path().into(), "workstation", &[], &Default::default()).unwrap();
    let item = |kind, change, subject: &str| Item::new(kind, change, subject, "");
    let mut items = vec![
        in_section(item(Kind::Package, Change::Added, "b"), Section::Fleet),
        item(Kind::Etc, Change::Edited, "/etc/b"),
        item(Kind::Package, Change::Removed, "a"),
        item(Kind::Etc, Change::Added, "/etc/z"),
        item(Kind::Package, Change::Added, "c"),
        item(Kind::Home, Change::Unreadable, "~/x"),
        item(Kind::Package, Change::Added, "a"),
    ];
    sort(&fleet, &mut items);
    let order: Vec<&str> = items.iter().map(|i| i.subject.as_str()).collect();
    assert_eq!(order, ["~/x", "a", "c", "a", "/etc/z", "/etc/b", "b"]);
}
