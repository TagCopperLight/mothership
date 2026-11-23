use super::*;
use std::os::unix::fs::symlink;

const TAG: &str = "types/workstation/users/tag/home";
const DEV: &str = "types/server/users/dev/home";

fn home(type_name: &str, user: &str) -> Reader {
    Reader::Home(Home {
        type_name: type_name.into(),
        user: user.into(),
    })
}

fn fleet() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for folder in [
        "shared/dot_config".to_owned(),
        format!("{TAG}/dot_config"),
        format!("{DEV}/dot_config"),
    ] {
        fs::create_dir_all(dir.path().join(folder)).unwrap();
    }
    fs::write(dir.path().join("shared/dot_gitconfig"), "shared").unwrap();
    fs::write(dir.path().join(format!("{TAG}/dot_own")), "tag's").unwrap();
    for home in [TAG, DEV] {
        symlink(
            "../../../../../shared/dot_gitconfig",
            dir.path().join(format!("{home}/dot_gitconfig")),
        )
        .unwrap();
    }
    dir
}

#[test]
fn a_shared_file_goes_to_every_home_linking_it() {
    let dir = fleet();
    check(dir.path()).unwrap();
    let homes = owners(dir.path(), &dir.path().join(DEV).join("dot_gitconfig")).unwrap();
    assert_eq!(homes, [home("server", "dev"), home("workstation", "tag")]);
}

#[test]
fn a_homes_own_file_goes_to_that_home_only() {
    let dir = fleet();
    let tag = [home("workstation", "tag")];
    assert_eq!(
        owners(dir.path(), &dir.path().join(TAG).join("dot_own")).unwrap(),
        tag
    );
    assert_eq!(
        owners(dir.path(), &dir.path().join(TAG).join("dot_new")).unwrap(),
        tag
    );
    assert_eq!(owners(dir.path(), &dir.path().join(TAG)).unwrap(), tag);
}

#[test]
fn the_shared_files_name_the_homes_linking_them() {
    let dir = fleet();
    fs::write(dir.path().join("shared/dot_config/unused"), "").unwrap();
    let shared = shared(dir.path()).unwrap();
    let found: Vec<(String, Vec<String>)> = shared
        .iter()
        .map(|file| {
            let homes = file.readers.iter().map(Reader::to_string).collect();
            (shown(&file.rel), homes)
        })
        .collect();
    assert_eq!(
        found,
        [
            ("~/.config/unused".to_owned(), vec![]),
            (
                "~/.gitconfig".to_owned(),
                vec!["server/dev".to_owned(), "workstation/tag".to_owned()]
            ),
        ]
    );
}

#[test]
fn a_secret_keeps_its_name_in_the_fleet() {
    assert_eq!(
        shown(Path::new(".secrets/proxy-key.age")),
        ".secrets/proxy-key.age"
    );
    assert_eq!(
        shown(Path::new("private_dot_ssh/encrypted_private_id.age")),
        "~/.ssh/id"
    );
}

#[test]
fn a_link_to_a_folder_is_refused() {
    let dir = fleet();
    symlink(
        "../../../../../../shared/dot_config",
        dir.path().join(DEV).join("dot_config/all"),
    )
    .unwrap();
    let error = check(dir.path()).unwrap_err().to_string();
    assert!(error.contains("leads to a folder"), "{error}");
}

#[test]
fn a_broken_link_is_refused() {
    let dir = fleet();
    symlink(
        "../../../../../shared/dot_gone",
        dir.path().join(DEV).join("dot_gone"),
    )
    .unwrap();
    let error = check(dir.path()).unwrap_err().to_string();
    assert!(error.contains("leads nowhere"), "{error}");
}

#[test]
fn a_link_out_of_shared_is_refused() {
    let dir = fleet();
    symlink(
        "../../../../workstation/users/tag/home/dot_own",
        dir.path().join(DEV).join("dot_own"),
    )
    .unwrap();
    let error = check(dir.path()).unwrap_err().to_string();
    assert!(error.contains("leads out of shared/"), "{error}");
}

fn with_root_links(dir: &Path) {
    fs::create_dir_all(dir.join("shared/.secrets")).unwrap();
    fs::write(dir.join("shared/.secrets/token.age"), "sealed").unwrap();
    fs::create_dir_all(dir.join(format!("{DEV}/.secrets"))).unwrap();
    symlink(
        "../../../../../../shared/.secrets/token.age",
        dir.join(format!("{DEV}/.secrets/token.age")),
    )
    .unwrap();
    let probe = dir.join("types/server/etc/mothership/probe");
    fs::create_dir_all(&probe).unwrap();
    symlink(
        "../../../../../shared/.secrets/token.age",
        probe.join("github-token.age"),
    )
    .unwrap();
    let services = dir.join("types/server/services");
    fs::create_dir_all(&services).unwrap();
    symlink(
        "../../../shared/.secrets/token.age",
        services.join("ghcr-token.age"),
    )
    .unwrap();
    symlink(
        "/usr/share/zoneinfo/UTC",
        dir.join("types/server/etc/localtime"),
    )
    .unwrap();
}

#[test]
fn root_reads_a_shared_secret_it_links_from_etc_or_its_services() {
    let dir = fleet();
    with_root_links(dir.path());
    check(dir.path()).unwrap();
    let link = dir
        .path()
        .join("types/server/etc/mothership/probe/github-token.age");
    assert_eq!(
        owners(dir.path(), &link).unwrap(),
        [Reader::Root("server".into()), home("server", "dev")]
    );
    let token = shared(dir.path())
        .unwrap()
        .into_iter()
        .find(|file| file.rel == Path::new(".secrets/token.age"))
        .unwrap();
    assert_eq!(
        token.readers,
        [Reader::Root("server".into()), home("server", "dev")]
    );
    assert_eq!(
        Reader::Root("server".into()).recipients(Path::new("f")),
        Path::new("f/types/server/recipients.txt")
    );
}

#[test]
fn root_links_only_secrets() {
    let dir = fleet();
    let etc = dir.path().join("types/server/etc");
    fs::create_dir_all(&etc).unwrap();
    symlink("../../../shared/dot_gitconfig", etc.join("gitconfig.age")).unwrap();
    let error = check(dir.path()).unwrap_err().to_string();
    assert!(error.contains("root links only shared secrets"), "{error}");
}
