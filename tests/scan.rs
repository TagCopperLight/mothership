use super::*;
use crate::item::Section;

fn in_section(mut item: Item, section: Section) -> Item {
    item.section = section;
    item
}

#[test]
fn items_sort_by_section_area_change_then_subject() {
    let dir = tempfile::TempDir::new().unwrap();
    let type_dir = dir.path().join("types/workstation");
    fs::create_dir_all(&type_dir).unwrap();
    fs::write(type_dir.join(crate::fleet::SYSTEM_TOML), "").unwrap();
    let repo = crate::fleet::Repo::new(dir.path().into(), "workstation", &[], &Default::default())
        .unwrap();
    let fleet = Fleet::of(
        repo,
        &crate::user::by_uid(unsafe { libc::geteuid() }).unwrap(),
    );
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
