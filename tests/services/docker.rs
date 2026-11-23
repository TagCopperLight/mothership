use super::*;
use std::{fs, process::Command};

fn run(script: &str, data: &Path, snapshot: &Path, parts: &[&str]) -> bool {
    Command::new("sh")
        .arg("-euc")
        .arg(format!("{PARTS}{script}"))
        .arg("sh")
        .arg(data)
        .arg(snapshot)
        .args(parts)
        .status()
        .unwrap()
        .success()
}

fn read(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

#[test]
fn a_snapshot_copies_its_parts_and_restores_only_them() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (data, snapshot) = (tmp.path().join("data"), tmp.path().join("snapshot"));
    fs::create_dir_all(data.join("db")).unwrap();
    fs::create_dir_all(data.join("conf")).unwrap();
    fs::write(data.join("db/rows"), "old").unwrap();
    fs::write(data.join("conf/keys"), "old").unwrap();
    fs::write(data.join("photo"), "kept").unwrap();
    assert!(run(
        SNAPSHOT,
        &data,
        &snapshot,
        &["db", "conf/keys", "missing"]
    ));
    assert!(read(&snapshot.join("photo")).is_none());

    fs::write(data.join("db/rows"), "migrated").unwrap();
    fs::write(data.join("db/new"), "migrated").unwrap();
    fs::write(data.join("conf/keys"), "new").unwrap();
    fs::write(data.join("missing"), "new").unwrap();
    fs::write(data.join("photo"), "new photo").unwrap();
    assert!(run(
        RESTORE,
        &data,
        &snapshot,
        &["db", "conf/keys", "missing"]
    ));
    assert_eq!(read(&data.join("db/rows")).as_deref(), Some("old"));
    assert!(read(&data.join("db/new")).is_none());
    assert_eq!(read(&data.join("conf/keys")).as_deref(), Some("old"));
    assert!(read(&data.join("missing")).is_none());
    assert_eq!(read(&data.join("photo")).as_deref(), Some("new photo"));
}

#[test]
fn without_parts_the_snapshot_covers_the_whole_folder() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (data, snapshot) = (tmp.path().join("data"), tmp.path().join("snapshot"));
    fs::create_dir_all(data.join("db")).unwrap();
    fs::write(data.join("db/rows"), "old").unwrap();
    assert!(run(SNAPSHOT, &data, &snapshot, &[]));
    fs::write(data.join("db/rows"), "new").unwrap();
    fs::write(data.join("extra"), "new").unwrap();
    assert!(run(RESTORE, &data, &snapshot, &[]));
    assert_eq!(read(&data.join("db/rows")).as_deref(), Some("old"));
    assert!(read(&data.join("extra")).is_none());
}

#[test]
fn a_part_behind_a_link_stays_where_it_is() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (data, snapshot) = (tmp.path().join("data"), tmp.path().join("snapshot"));
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&data).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret"), "outside").unwrap();
    std::os::unix::fs::symlink(&outside, data.join("db")).unwrap();
    assert!(!run(SNAPSHOT, &data, &snapshot, &["db/secret"]));
    assert!(!run(RESTORE, &data, &snapshot, &["db/secret"]));
    assert_eq!(read(&outside.join("secret")).as_deref(), Some("outside"));
}

fn container(status: &str, exit_code: i64, restart_count: i64) -> Container {
    serde_json::from_value(serde_json::json!({
        "Id": "c1",
        "Name": "/app-web-1",
        "Image": "sha256:0",
        "State": {"Status": status, "ExitCode": exit_code, "Health": null},
        "RestartCount": restart_count,
        "Config": {"Labels": null},
    }))
    .unwrap()
}

#[test]
fn a_restart_during_the_watch_is_a_crash_and_an_older_one_isnt() {
    let before = BTreeMap::from([("c1".to_owned(), 2)]);
    assert_eq!(sick(&[container("running", 0, 2)], &before), None);
    assert_eq!(
        sick(&[container("running", 0, 3)], &before).as_deref(),
        Some("app-web-1 crashed and restarted")
    );
}

#[test]
fn only_a_failed_exit_is_a_crash() {
    let before = BTreeMap::from([("c1".to_owned(), 0)]);
    assert_eq!(sick(&[container("exited", 0, 0)], &before), None);
    assert_eq!(
        sick(&[container("exited", 1, 0)], &before).as_deref(),
        Some("app-web-1 stopped with code 1")
    );
}
