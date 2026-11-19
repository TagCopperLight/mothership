use super::*;
use std::fs;

#[test]
fn an_ignored_folder_ignores_what_it_holds_but_not_its_neighbours() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ignore.txt");
    fs::write(&path, "home ~/.config\n").unwrap();
    let ignore = Ignore::load(&path, Level::User).unwrap();
    assert!(ignore.matches_subject(Kind::Home, "~/.config"));
    assert!(ignore.matches_subject(Kind::Home, "~/.config/app/settings.json"));
    assert!(!ignore.matches_subject(Kind::Home, "~/.configs"));
    assert!(!ignore.matches_subject(Kind::Home, "~/.local/share"));
    assert!(!ignore.matches_subject(Kind::File, "~/.config/app"));
}

#[test]
fn a_kind_reads_back_from_its_name() {
    assert_eq!(Kind::UserService.name(), "user-service");
    assert!("user-service".parse::<Kind>() == Ok(Kind::UserService));
    assert!("userservice".parse::<Kind>().is_err());
}

fn load(text: &str, level: Level) -> Result<Ignore> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ignore.txt");
    fs::write(&path, text).unwrap();
    Ignore::load(&path, level)
}

#[test]
fn a_line_review_writes_matches_its_item_even_with_glob_characters() {
    let item = Item::new(
        Kind::Home,
        crate::scan::Change::Added,
        "~/.config/a[1]*",
        "new",
    );
    let ignore = load(&crate::scan::ignore_line(&item), Level::User).unwrap();
    assert!(ignore.matches(&item));
    assert!(!ignore.matches_subject(Kind::Home, "~/.config/a1x"));
}

#[test]
fn a_star_stops_at_a_slash_and_comments_are_skipped() {
    let ignore = load("# caches\n\nhome ~/*.log\n", Level::User).unwrap();
    assert!(ignore.matches_subject(Kind::Home, "~/app.log"));
    assert!(!ignore.matches_subject(Kind::Home, "~/.config/app.log"));
    let missing = Ignore::load(Path::new("/nonexistent/ignore.txt"), Level::User).unwrap();
    assert!(!missing.matches_subject(Kind::Home, "~/app.log"));
}

#[test]
fn a_malformed_line_names_its_place() {
    let error = |text: &str| format!("{:#}", load(text, Level::Machine).err().unwrap());
    assert!(error("package").ends_with(":1: expected `<kind> <glob>`"));
    assert!(error("# x\nwidget htop").ends_with(":2: unknown kind widget"));
    assert!(error("package [").contains("ignore.txt:1"));
}
