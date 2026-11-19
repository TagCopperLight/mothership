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
