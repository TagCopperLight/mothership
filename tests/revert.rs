use super::*;
use tempfile::TempDir;

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn links_over_a_file_and_leaves_no_temporary_link() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("resolv.conf");
    fs::write(&path, "nameserver 1.1.1.1\n").unwrap();
    link(&path, Path::new("/run/systemd/resolve/stub-resolv.conf")).unwrap();
    assert_eq!(
        fs::read_link(&path).unwrap(),
        Path::new("/run/systemd/resolve/stub-resolv.conf")
    );
    link(&path, Path::new("/run/other")).unwrap();
    assert_eq!(fs::read_link(&path).unwrap(), Path::new("/run/other"));
    assert_eq!(names(dir.path()), ["resolv.conf"]);
}
