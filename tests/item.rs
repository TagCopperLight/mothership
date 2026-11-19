use super::*;
use crate::{item::Item, revert, secrets::Storage};

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
