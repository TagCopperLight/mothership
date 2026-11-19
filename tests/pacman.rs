use super::*;

#[test]
fn reads_names_without_their_version() {
    let desc = "%DEPENDS%\nglibc>=2.40\nsh\nzlib=1.3\n\n%PROVIDES%\nlibfoo.so=1-64\n";
    assert_eq!(names(desc, "DEPENDS"), ["glibc", "sh", "zlib"]);
    assert_eq!(names(desc, "PROVIDES"), ["libfoo.so"]);
}

#[test]
fn installs_repo_packages_with_pacman_and_the_rest_with_the_aur_helper() {
    let in_repos = BTreeSet::from(["vim".to_owned(), "zip".to_owned()]);
    let install =
        split_install(&["vim", "yay", "zip"], &in_repos, Some(aur::Helper::Paru)).unwrap();
    assert_eq!(
        install,
        Install {
            repo: vec!["vim", "zip"],
            aur: Some((aur::Helper::Paru, vec!["yay"])),
        }
    );
    let install = split_install(&["vim"], &in_repos, None).unwrap();
    assert_eq!(
        install,
        Install {
            repo: vec!["vim"],
            aur: None,
        }
    );
}

#[test]
fn aur_packages_without_a_helper_fail_before_anything_installs() {
    let in_repos = BTreeSet::from(["vim".to_owned()]);
    let error = split_install(&["vim", "yay"], &in_repos, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "yay not in pacman's repos, and system.toml sets no aur_helper"
    );
}

#[test]
fn a_package_entry_missing_its_version_or_a_backup_hash_fails() {
    let desc = "%NAME%\nvim\n\n%VERSION%\n9.1-1\n\n%ARCH%\nx86_64\n";
    let files = "%FILES%\nusr/bin/vim\n\n%BACKUP%\netc/vimrc\t0123\n";
    let package = parse(desc, files).unwrap();
    assert_eq!(
        package.archive.path,
        Path::new(CACHE).join("vim-9.1-1-x86_64.pkg.tar.zst")
    );
    assert_eq!(package.backups[0].path, Path::new("/etc/vimrc"));
    let no_version = desc.replace("%VERSION%\n9.1-1\n\n", "");
    assert!(parse(&no_version, files).is_err());
    assert!(parse(desc, "%BACKUP%\netc/vimrc\n").is_err());
}
