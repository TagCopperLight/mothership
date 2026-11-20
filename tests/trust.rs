use super::*;
use crate::secrets::names;
use std::{fs, os::unix::fs::symlink};

#[test]
fn home_secrets_are_found_by_chezmoi_names() {
    let shown = |rel: &str| home_secret(Path::new(rel));
    assert_eq!(
        shown("private_dot_ssh/encrypted_private_id_ed25519.age").as_deref(),
        Some("~/.ssh/id_ed25519")
    );
    assert_eq!(
        shown("dot_config/app/encrypted_token.tmpl.age").as_deref(),
        Some("~/.config/app/token")
    );
    assert_eq!(shown("dot_config/app/settings.json"), None);
    assert_eq!(
        shown(".secrets/proxy-key.age").as_deref(),
        Some(".secrets/proxy-key.age")
    );
    assert_eq!(shown(".secrets/notes.txt"), None);
}

#[test]
fn a_shared_home_file_is_for_the_list_of_every_home_linking_it() {
    let dir = tempfile::TempDir::new().unwrap();
    let fleet = dir.path();
    for (type_name, user, text) in [
        (
            "workstation",
            "tag",
            "# laptop\nage1tag\n\n# recovery\nage1rec\n",
        ),
        (
            "server",
            "dev",
            "# minipc\nage1dev\n\n# recovery\nage1rec\n",
        ),
    ] {
        let users = fleet.join("types").join(type_name).join("users").join(user);
        fs::create_dir_all(users.join("home")).unwrap();
        fs::write(users.join(RECIPIENTS), text).unwrap();
        symlink(
            "../../../../../shared/dot_token",
            users.join("home/dot_token"),
        )
        .unwrap();
    }
    let elsewhere = fleet.join("types/laptop/users/dev");
    fs::create_dir_all(elsewhere.join("home")).unwrap();
    fs::write(elsewhere.join(RECIPIENTS), "# other\nage1other\n").unwrap();
    fs::create_dir_all(fleet.join("shared")).unwrap();
    fs::write(fleet.join("shared/dot_token"), "").unwrap();
    let keys = |list: Vec<Recipient>| list.into_iter().map(|r| r.key).collect::<Vec<_>>();
    let link = fleet.join("types/server/users/dev/home/dot_token");
    assert_eq!(
        keys(home_recipients(fleet, &link, None).unwrap()),
        ["age1dev", "age1rec", "age1tag"]
    );
    let dev_list = fleet.join("types/server/users/dev").join(RECIPIENTS);
    let new = [Recipient {
        name: "minipc".into(),
        key: "age1new".into(),
    }];
    assert_eq!(
        keys(home_recipients(fleet, &link, Some((&dev_list, &new))).unwrap()),
        ["age1new", "age1tag", "age1rec"]
    );
}

fn fleet_with(dir: &Path, list: &str, secrets: &[(&str, &[u8], &Path)]) -> Fleet {
    let type_dir = dir.join("types/workstation");
    fs::create_dir_all(type_dir.join("etc")).unwrap();
    fs::write(type_dir.join(fleet::SYSTEM_TOML), "").unwrap();
    fs::write(type_dir.join(RECIPIENTS), list).unwrap();
    for (name, text, sealed_for) in secrets {
        let sealed = crate::support::seal(sealed_for, text);
        fs::write(type_dir.join("etc").join(name), sealed).unwrap();
    }
    crate::support::name_recovery(dir);
    let repo = crate::fleet::Repo::new(dir.to_path_buf(), "workstation", &[], &Default::default())
        .unwrap();
    Fleet::of(repo, &crate::support::current_user())
}

#[test]
fn trusting_a_key_opens_every_secret_of_its_list_to_it() {
    let dir = tempfile::TempDir::new().unwrap();
    let laptop = make_key(&dir.path().join("laptop.txt")).unwrap();
    let desktop = make_key(&dir.path().join("desktop.txt")).unwrap();
    let list = dir.path().join("list.txt");
    fs::write(&list, format!("# laptop\n{}\n", public(&laptop))).unwrap();
    let fleet = fleet_with(
        dir.path(),
        &fs::read_to_string(&list).unwrap(),
        &[("wifi.conf.age", b"psk=1", &list)],
    );
    let mut keys = Keys::default();
    keys.push(laptop);
    trust_with(
        &fleet,
        &keys,
        "workstation/root",
        "desktop",
        &public(&desktop),
        &mut Prompt::answering("y"),
    )
    .unwrap();
    assert_eq!(names(&fleet.recipients()).unwrap(), ["laptop", "desktop"]);
    let mut theirs = Keys::default();
    theirs.push(desktop);
    let sealed = fleet.etc_dir().join("wifi.conf.age");
    assert_eq!(theirs.decrypt(&sealed).unwrap(), b"psk=1");
}

#[test]
fn a_secret_this_machine_can_not_open_stops_trust_before_any_write() {
    let dir = tempfile::TempDir::new().unwrap();
    let laptop = make_key(&dir.path().join("laptop.txt")).unwrap();
    let stranger = make_key(&dir.path().join("stranger.txt")).unwrap();
    let (mine, theirs) = (dir.path().join("mine.txt"), dir.path().join("theirs.txt"));
    fs::write(&mine, format!("# laptop\n{}\n", public(&laptop))).unwrap();
    fs::write(&theirs, format!("# stranger\n{}\n", public(&stranger))).unwrap();
    let fleet = fleet_with(
        dir.path(),
        &fs::read_to_string(&mine).unwrap(),
        &[("a.conf.age", b"a", &mine), ("b.conf.age", b"b", &theirs)],
    );
    let before = |name: &str| fs::read(fleet.etc_dir().join(name)).unwrap();
    let (a, list) = (before("a.conf.age"), fs::read(fleet.recipients()).unwrap());
    let mut keys = Keys::default();
    keys.push(laptop);
    let other = make_key(&dir.path().join("other.txt")).unwrap();
    let error = trust_with(
        &fleet,
        &keys,
        "workstation/root",
        "desktop",
        &public(&other),
        &mut Prompt::answering("y"),
    );
    let error = format!("{:#}", error.err().unwrap());
    assert!(error.starts_with("none of this machine's keys opens"));
    assert!(error.ends_with("etc/b.conf.age"));
    assert_eq!(before("a.conf.age"), a);
    assert_eq!(fs::read(fleet.recipients()).unwrap(), list);
}

#[test]
fn a_name_the_list_holds_is_refused_until_it_is_retired() {
    let dir = tempfile::TempDir::new().unwrap();
    let laptop = make_key(&dir.path().join("laptop.txt")).unwrap();
    let other = make_key(&dir.path().join("other.txt")).unwrap();
    let list = format!("# laptop\n{}\n", public(&laptop));
    let fleet = fleet_with(dir.path(), &list, &[]);
    let mut keys = Keys::default();
    keys.push(laptop);
    let error = trust_with(
        &fleet,
        &keys,
        "workstation/root",
        "laptop",
        &public(&other),
        &mut Prompt::answering("y"),
    );
    let error = error.err().unwrap().to_string();
    assert!(error.contains("mothership retire workstation/root laptop"));
    assert_eq!(fs::read_to_string(fleet.recipients()).unwrap(), list);
}

struct SharedSecret {
    _dir: tempfile::TempDir,
    fleet: Fleet,
    laptop: x25519::Identity,
    minipc: x25519::Identity,
    file: PathBuf,
}

fn shared_secret() -> SharedSecret {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();
    let laptop = make_key(&root.join("laptop.txt")).unwrap();
    let minipc = make_key(&root.join("minipc.txt")).unwrap();
    for (type_name, user, key) in [("workstation", "tag", &laptop), ("server", "dev", &minipc)] {
        let users = root.join("types").join(type_name).join("users").join(user);
        fs::create_dir_all(users.join("home/.secrets")).unwrap();
        fs::write(users.join(RECIPIENTS), format!("# key\n{}\n", public(key))).unwrap();
        fs::write(users.join(fleet::USER_TOML), "").unwrap();
        symlink(
            "../../../../../../shared/.secrets/proxy-key.age",
            users.join("home/.secrets/proxy-key.age"),
        )
        .unwrap();
    }
    let both = root.join("both.txt");
    let lists = format!(
        "# laptop\n{}\n# minipc\n{}\n",
        public(&laptop),
        public(&minipc)
    );
    fs::write(&both, lists).unwrap();
    fs::create_dir_all(root.join("shared/.secrets")).unwrap();
    let file = root.join("shared/.secrets/proxy-key.age");
    fs::write(&file, crate::support::seal(&both, b"proxy")).unwrap();
    for type_name in ["workstation", "server"] {
        fs::write(
            root.join("types").join(type_name).join(fleet::SYSTEM_TOML),
            "",
        )
        .unwrap();
    }
    crate::support::name_recovery(root);
    let repo = crate::fleet::Repo::new(root.to_path_buf(), "workstation", &[], &Default::default())
        .unwrap();
    SharedSecret {
        fleet: Fleet::of(repo, &crate::support::current_user()),
        _dir: dir,
        laptop,
        minipc,
        file,
    }
}

fn opens(key: &x25519::Identity, file: &Path) -> bool {
    let mut keys = Keys::default();
    keys.push(key.clone());
    keys.decrypt(file).is_ok()
}

#[test]
fn trusting_a_key_for_one_home_keeps_a_shared_secret_open_to_the_other_homes() {
    let shared = shared_secret();
    let desktop = x25519::Identity::generate();
    let mut keys = Keys::default();
    keys.push(shared.laptop.clone());
    trust_with(
        &shared.fleet,
        &keys,
        "workstation/tag",
        "desktop",
        &public(&desktop),
        &mut Prompt::answering("y"),
    )
    .unwrap();
    assert!(opens(&desktop, &shared.file));
    assert!(opens(&shared.laptop, &shared.file));
    assert!(opens(&shared.minipc, &shared.file));
}

#[test]
fn saying_no_to_trust_changes_nothing() {
    let shared = shared_secret();
    let before = fs::read(&shared.file).unwrap();
    let list = shared
        .fleet
        .dir
        .join("types/workstation/users/tag")
        .join(RECIPIENTS);
    let names_before = names(&list).unwrap();
    let mut keys = Keys::default();
    keys.push(shared.laptop.clone());
    trust_with(
        &shared.fleet,
        &keys,
        "workstation/tag",
        "desktop",
        &public(&x25519::Identity::generate()),
        &mut Prompt::answering("n"),
    )
    .unwrap();
    assert_eq!(fs::read(&shared.file).unwrap(), before);
    assert_eq!(names(&list).unwrap(), names_before);
}
