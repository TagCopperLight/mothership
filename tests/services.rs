use super::*;

fn unit(dir: &Path, name: &str, text: &str) {
    fs::write(dir.join(name), text).unwrap();
}

#[test]
fn also_follows_install_lines_through_every_unit() {
    let dir = tempfile::tempdir().unwrap();
    let dirs = [dir.path().to_path_buf()];
    unit(
        dir.path(),
        "a.service",
        "[Unit]\nAlso=ignored.service\n[Install]\nAlso=b.service c.socket\n",
    );
    unit(dir.path(), "b.service", "[Install]\nAlso=d.socket\n");
    let covered = with_also(&["a.service".to_owned()], &dirs).unwrap();
    let expected = ["a.service", "b.service", "c.socket", "d.socket"];
    assert_eq!(covered, expected.iter().map(|s| s.to_string()).collect());
}

#[test]
fn an_earlier_folder_hides_the_same_unit_in_a_later_one() {
    let (etc, lib) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    unit(etc.path(), "a.service", "[Install]\n");
    unit(lib.path(), "a.service", "[Install]\nAlso=b.service\n");
    let dirs = [etc.path().to_path_buf(), lib.path().to_path_buf()];
    assert!(also("a.service", &dirs).unwrap().is_empty());
}
