use super::*;
use std::os::unix::fs::symlink;

fn fleet() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for folder in [
        "shared/dot_config",
        "homes/tag/dot_config",
        "homes/dev/dot_config",
    ] {
        fs::create_dir_all(dir.path().join(folder)).unwrap();
    }
    fs::write(dir.path().join("shared/dot_gitconfig"), "shared").unwrap();
    fs::write(dir.path().join("homes/tag/dot_own"), "tag's").unwrap();
    for user in ["tag", "dev"] {
        symlink(
            "../../shared/dot_gitconfig",
            dir.path().join(format!("homes/{user}/dot_gitconfig")),
        )
        .unwrap();
    }
    dir
}

#[test]
fn a_shared_file_goes_to_every_user_linking_it() {
    let dir = fleet();
    check(dir.path()).unwrap();
    let users = users(dir.path(), &dir.path().join("homes/dev/dot_gitconfig")).unwrap();
    assert_eq!(users, ["dev", "tag"]);
}

#[test]
fn a_users_own_file_goes_to_that_user_only() {
    let dir = fleet();
    assert_eq!(
        users(dir.path(), &dir.path().join("homes/tag/dot_own")).unwrap(),
        ["tag"]
    );
    assert_eq!(
        users(dir.path(), &dir.path().join("homes/tag/dot_new")).unwrap(),
        ["tag"]
    );
}

#[test]
fn a_link_to_a_folder_is_refused() {
    let dir = fleet();
    symlink(
        "../../../shared/dot_config",
        dir.path().join("homes/dev/dot_config/all"),
    )
    .unwrap();
    let error = check(dir.path()).unwrap_err().to_string();
    assert!(error.contains("leads to a folder"), "{error}");
}

#[test]
fn a_broken_link_is_refused() {
    let dir = fleet();
    symlink(
        "../../shared/dot_gone",
        dir.path().join("homes/dev/dot_gone"),
    )
    .unwrap();
    let error = check(dir.path()).unwrap_err().to_string();
    assert!(error.contains("leads nowhere"), "{error}");
}

#[test]
fn a_link_out_of_shared_is_refused() {
    let dir = fleet();
    symlink("../tag/dot_own", dir.path().join("homes/dev/dot_own")).unwrap();
    let error = check(dir.path()).unwrap_err().to_string();
    assert!(error.contains("leads out of shared/"), "{error}");
}
