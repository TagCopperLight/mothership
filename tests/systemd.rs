use super::*;
use std::os::unix::fs::symlink;

fn unit(dir: &Path, name: &str, text: &str) {
    fs::write(dir.join(name), text).unwrap();
}

#[test]
fn an_enabled_instance_keeps_its_name_and_a_mask_enables_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let wants = dir.path().join("getty.target.wants");
    fs::create_dir(&wants).unwrap();
    symlink("/usr/lib/getty@.service", wants.join("getty@tty1.service")).unwrap();
    symlink("/dev/null", dir.path().join("a.service")).unwrap();
    let units = enabled(&[&dir.path().to_path_buf()], &[]).unwrap();
    assert_eq!(units, BTreeSet::from(["getty@tty1.service".to_owned()]));
}

#[test]
fn a_requires_folder_and_an_alias_enable_their_units() {
    let dir = tempfile::tempdir().unwrap();
    let requires = dir.path().join("multi-user.target.requires");
    fs::create_dir(&requires).unwrap();
    symlink("/usr/lib/a.service", requires.join("a.service")).unwrap();
    symlink(
        "/usr/lib/gdm3.service",
        dir.path().join("display-manager.service"),
    )
    .unwrap();
    let units = enabled(&[&dir.path().to_path_buf()], &[]).unwrap();
    let expected = ["a.service", "gdm3.service"].map(str::to_owned);
    assert_eq!(units, BTreeSet::from(expected));
}

#[test]
fn a_link_named_like_its_target_or_with_no_file_name_enables_nothing() {
    let dir = tempfile::tempdir().unwrap();
    symlink("/opt/app/app.service", dir.path().join("app.service")).unwrap();
    symlink("..", dir.path().join("x")).unwrap();
    let units = enabled(&[&dir.path().to_path_buf()], &[]).unwrap();
    assert!(units.is_empty());
}

#[test]
fn an_alias_resolves_to_the_unit_it_names() {
    let (etc, lib) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    unit(lib.path(), "ssh.service", "[Install]\n");
    symlink("ssh.service", lib.path().join("sshd.service")).unwrap();
    symlink(
        lib.path().join("sshd.service"),
        etc.path().join("ssh-alias.service"),
    )
    .unwrap();
    let dirs = [etc.path().to_path_buf(), lib.path().to_path_buf()];
    assert_eq!(real_name("sshd.service", &dirs).unwrap(), "ssh.service");
    assert_eq!(
        real_name("ssh-alias.service", &dirs).unwrap(),
        "ssh.service"
    );
    assert_eq!(real_name("ssh.service", &dirs).unwrap(), "ssh.service");
    assert_eq!(real_name("gone.service", &dirs).unwrap(), "gone.service");
}

#[test]
fn an_instance_takes_its_templates_file() {
    let dir = tempfile::tempdir().unwrap();
    unit(dir.path(), "getty@.service", "[Install]\n");
    let dirs = [dir.path().to_path_buf()];
    assert_eq!(
        unit_file("getty@tty1.service", &dirs),
        Some(dir.path().join("getty@.service"))
    );
}
