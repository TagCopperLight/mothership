use super::*;
use crate::item::Section;

use crate::support::in_section;

#[test]
fn items_sort_by_section_area_change_then_subject() {
    let dir = tempfile::TempDir::new().unwrap();
    let fleet = crate::support::empty_fleet(dir.path());
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
