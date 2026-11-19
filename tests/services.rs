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

fn presets(text: &str) -> Presets {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("90-test.preset"), text).unwrap();
    Presets::load(&[dir.path().to_str().unwrap()]).unwrap()
}

#[test]
fn an_enabled_instance_keeps_its_name_and_a_mask_enables_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let wants = dir.path().join("getty.target.wants");
    fs::create_dir(&wants).unwrap();
    std::os::unix::fs::symlink("/usr/lib/getty@.service", wants.join("getty@tty1.service"))
        .unwrap();
    std::os::unix::fs::symlink("/dev/null", dir.path().join("a.service")).unwrap();
    let units = enabled(&[&dir.path().to_path_buf()]).unwrap();
    assert_eq!(units, BTreeSet::from(["getty@tty1.service".to_owned()]));
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

#[test]
fn a_template_preset_enables_its_default_instance_or_the_listed_ones() {
    let dir = tempfile::tempdir().unwrap();
    unit(
        dir.path(),
        "getty@.service",
        "[Install]\nDefaultInstance=tty1\n",
    );
    let dirs = [dir.path().to_path_buf()];
    let rules = presets("enable getty@.service\nenable serial@.service ttyS0\ndisable *\n");
    let enables = |name| rules.enables(name, &dirs).unwrap();
    assert!(enables("getty@tty1.service"));
    assert!(!enables("getty@tty2.service"));
    assert!(enables("serial@ttyS0.service"));
    assert!(!enables("serial@ttyS1.service"));
}

#[test]
fn on_debian_the_units_a_package_enabled_are_its_defaults() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("sysinit.target.wants")).unwrap();
    fs::write(dir.path().join("sysinit.target.wants/apparmor.service"), "").unwrap();
    fs::write(dir.path().join("ssh.service.dsh-also"), "").unwrap();
    let units = packaged_units(dir.path()).unwrap();
    let expected = ["apparmor.service", "ssh.service"].map(str::to_owned);
    assert_eq!(units, BTreeSet::from(expected));
}
