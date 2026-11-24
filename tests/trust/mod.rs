use super::*;
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

fn keys_of(list: Vec<Recipient>) -> Vec<String> {
    list.into_iter().map(|r| r.key).collect()
}

#[test]
fn a_shared_home_file_is_for_the_list_of_every_home_linking_it() {
    let dir = tempfile::TempDir::new().unwrap();
    let fleet = dir.path();
    for (machine, type_name, user, key) in [
        ("laptop", "workstation", "tag", "age1tag"),
        ("minipc", "server", "dev", "age1dev"),
        ("other", "laptop", "dev", "age1other"),
    ] {
        let users = fleet.join("types").join(type_name).join("users").join(user);
        fs::create_dir_all(users.join("home")).unwrap();
        let text = format!("type = \"{type_name}\"\n[users.{user}]\nage = \"{key}\"\n");
        crate::support::machine(fleet, machine, &text);
        if machine != "other" {
            symlink(
                "../../../../../shared/dot_token",
                users.join("home/dot_token"),
            )
            .unwrap();
        }
    }
    fs::create_dir_all(fleet.join("shared")).unwrap();
    fs::write(fleet.join("shared/dot_token"), "").unwrap();
    let link = fleet.join("types/server/users/dev/home/dot_token");
    let readers = homes::owners(fleet, &link).unwrap();
    assert_eq!(
        keys_of(Machines::load(fleet).unwrap().recipients_of(&readers)),
        ["age1dev", "age1tag"]
    );
}

fn root_machine(key: &x25519::Identity) -> String {
    format!(
        "type = \"workstation\"\n[keys]\nage = \"{}\"\n",
        public(key)
    )
}

fn fleet_with(
    dir: &Path,
    machines: &[(&str, String)],
    secrets: &[(&str, &[u8], &[&x25519::Identity])],
) -> Fleet {
    let type_dir = dir.join("types/workstation");
    fs::create_dir_all(type_dir.join("etc")).unwrap();
    fs::write(type_dir.join(fleet::SYSTEM_TOML), "").unwrap();
    for (name, text) in machines {
        crate::support::machine(dir, name, text);
    }
    for (name, text, sealed_for) in secrets {
        let sealed = crate::support::seal(sealed_for, text);
        fs::write(type_dir.join("etc").join(name), sealed).unwrap();
    }
    crate::support::name_recovery(dir);
    let repo = crate::fleet::Repo::new(dir.to_path_buf(), "workstation", &[], &Default::default())
        .unwrap();
    Fleet::of(repo, &crate::support::current_user())
}

fn workstation() -> Machine {
    Machine {
        device_type: "workstation".into(),
        options: Vec::new(),
    }
}

fn keys(key: &x25519::Identity) -> Keys {
    let mut keys = Keys::default();
    keys.push(key.clone());
    keys
}

fn trust(fleet: &Fleet, keys: &Keys, host: &str, entries: &[String]) -> Result<Vec<github::Key>> {
    trust_host(
        fleet,
        keys,
        host,
        &workstation(),
        entries,
        &mut Prompt::answering("\n"),
    )
}

#[test]
fn trusting_a_machine_opens_every_secret_its_keys_read_to_it() {
    let dir = tempfile::TempDir::new().unwrap();
    let laptop = make_key(&dir.path().join("laptop.txt")).unwrap();
    let desktop = make_key(&dir.path().join("desktop.txt")).unwrap();
    let fleet = fleet_with(
        dir.path(),
        &[("laptop", root_machine(&laptop))],
        &[("wifi.conf.age", b"psk=1", &[&laptop])],
    );
    trust(
        &fleet,
        &keys(&laptop),
        "desktop",
        &[format!("root={}", public(&desktop))],
    )
    .unwrap();
    let written = Machines::load(dir.path()).unwrap();
    assert_eq!(
        written.get("desktop").unwrap().keys.age,
        Some(public(&desktop))
    );
    let sealed = fleet.etc_dir().join("wifi.conf.age");
    assert_eq!(keys(&desktop).decrypt(&sealed).unwrap(), b"psk=1");
}

#[test]
fn retire_drops_a_machine_without_opening_the_secrets() {
    let dir = tempfile::TempDir::new().unwrap();
    let laptop = make_key(&dir.path().join("laptop.txt")).unwrap();
    let desktop = make_key(&dir.path().join("desktop.txt")).unwrap();
    let fleet = fleet_with(
        dir.path(),
        &[
            ("laptop", root_machine(&laptop)),
            ("desktop", root_machine(&desktop)),
        ],
        &[("wifi.conf.age", b"psk=1", &[&laptop, &desktop])],
    );
    let sealed = fleet.etc_dir().join("wifi.conf.age");
    let before = fs::read(&sealed).unwrap();
    retire_host(&fleet, "laptop").unwrap();
    assert!(!machines::path(dir.path(), "laptop").exists());
    assert!(machines::path(dir.path(), "desktop").exists());
    assert_eq!(fs::read(&sealed).unwrap(), before);
}

#[test]
fn a_secret_this_machine_can_not_open_stops_trust_before_any_write() {
    let dir = tempfile::TempDir::new().unwrap();
    let laptop = make_key(&dir.path().join("laptop.txt")).unwrap();
    let stranger = make_key(&dir.path().join("stranger.txt")).unwrap();
    let fleet = fleet_with(
        dir.path(),
        &[("laptop", root_machine(&laptop))],
        &[
            ("a.conf.age", b"a", &[&laptop]),
            ("b.conf.age", b"b", &[&stranger]),
        ],
    );
    let before = |name: &str| fs::read(fleet.etc_dir().join(name)).unwrap();
    let a = before("a.conf.age");
    let other = make_key(&dir.path().join("other.txt")).unwrap();
    let error = trust(
        &fleet,
        &keys(&laptop),
        "desktop",
        &[format!("root={}", public(&other))],
    );
    let error = format!("{:#}", error.err().unwrap());
    assert!(error.starts_with("none of this machine's keys opens"));
    assert!(error.ends_with("etc/b.conf.age"));
    assert_eq!(before("a.conf.age"), a);
    assert!(!machines::path(dir.path(), "desktop").exists());
}

#[test]
fn a_machine_keeps_its_keys_and_type_until_it_is_retired() {
    let dir = tempfile::TempDir::new().unwrap();
    let laptop = make_key(&dir.path().join("laptop.txt")).unwrap();
    let other = make_key(&dir.path().join("other.txt")).unwrap();
    let fleet = fleet_with(dir.path(), &[("laptop", root_machine(&laptop))], &[]);
    let file = machines::path(dir.path(), "laptop");
    let text = fs::read_to_string(&file).unwrap();
    let error = trust(
        &fleet,
        &keys(&laptop),
        "laptop",
        &[format!("root={}", public(&other))],
    );
    assert_eq!(
        error.err().unwrap().to_string(),
        "machines/laptop.toml lists another age key of root for laptop: retire it first"
    );
    let server = Machine {
        device_type: "server".into(),
        options: Vec::new(),
    };
    let moved = trust_host(
        &fleet,
        &keys(&laptop),
        "laptop",
        &server,
        &[],
        &mut Prompt::answering(""),
    );
    assert!(
        moved
            .err()
            .unwrap()
            .to_string()
            .ends_with("change it there")
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), text);
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
    for (machine, type_name, user, key) in [
        ("laptop", "workstation", "tag", &laptop),
        ("minipc", "server", "dev", &minipc),
    ] {
        let users = root.join("types").join(type_name).join("users").join(user);
        fs::create_dir_all(users.join("home/.secrets")).unwrap();
        let text = format!(
            "type = \"{type_name}\"\n[users.{user}]\nage = \"{}\"\n",
            public(key)
        );
        crate::support::machine(root, machine, &text);
        fs::write(users.join(fleet::USER_TOML), "").unwrap();
        symlink(
            "../../../../../../shared/.secrets/proxy-key.age",
            users.join("home/.secrets/proxy-key.age"),
        )
        .unwrap();
    }
    fs::create_dir_all(root.join("shared/.secrets")).unwrap();
    let file = root.join("shared/.secrets/proxy-key.age");
    fs::write(&file, crate::support::seal(&[&laptop, &minipc], b"proxy")).unwrap();
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
    keys(key).decrypt(file).is_ok()
}

#[test]
fn trusting_a_key_for_one_home_keeps_a_shared_secret_open_to_the_other_homes() {
    let shared = shared_secret();
    let desktop = x25519::Identity::generate();
    let entries = [format!("tag={}", public(&desktop))];
    trust(&shared.fleet, &keys(&shared.laptop), "desktop", &entries).unwrap();
    assert!(opens(&desktop, &shared.file));
    assert!(opens(&shared.laptop, &shared.file));
    assert!(opens(&shared.minipc, &shared.file));
}

fn random() -> String {
    let key =
        ssh_key::PrivateKey::random(&mut ssh_key::rand_core::OsRng, ssh_key::Algorithm::Ed25519)
            .unwrap();
    sign::public(&key).unwrap()
}

#[test]
fn trust_lists_a_host_s_ssh_and_signing_keys_and_retire_drops_them() {
    let dir = tempfile::TempDir::new().unwrap();
    let user_dir = dir.path().join("types/workstation/users/me");
    fs::create_dir_all(user_dir.join("home")).unwrap();
    fs::write(user_dir.join(fleet::USER_TOML), "").unwrap();
    let fleet = fleet_with(dir.path(), &[], &[]);
    let (login, signing) = (random(), random());
    let entries = [
        format!("me={login} me@laptop"),
        format!("{SIGNING}={signing}"),
    ];
    let none = Keys::default();

    let uploads = trust(&fleet, &none, "desktop", &entries).unwrap();
    assert_eq!(
        uploads,
        [
            github::Key {
                title: "desktop/me".into(),
                public: login.clone(),
                kind: github::Use::Login,
            },
            github::Key {
                title: "desktop".into(),
                public: signing.clone(),
                kind: github::Use::Signing,
            },
        ]
    );
    let written = Machines::load(dir.path()).unwrap();
    let desktop = written.get("desktop").unwrap();
    assert_eq!(desktop.users["me"].ssh, Some(login));
    assert_eq!(desktop.keys.signing, Some(signing));
    assert_eq!(
        desktop.signs,
        Some(sign::Reach::Folders(vec!["types/workstation".into()]))
    );
    let fleet = fleet_with(dir.path(), &[], &[]);
    assert_eq!(trust(&fleet, &none, "desktop", &entries).unwrap().len(), 2);
    let error = trust(&fleet, &none, "desktop", &[format!("me={}", random())]);
    assert!(
        error
            .err()
            .unwrap()
            .to_string()
            .ends_with("lists another SSH key of me for desktop: retire it first")
    );

    retire_host(&fleet, "desktop").unwrap();
    assert!(!machines::path(dir.path(), "desktop").exists());
    let fleet = fleet_with(dir.path(), &[], &[]);
    assert!(retire_host(&fleet, "desktop").is_err());
}

#[test]
fn a_signer_may_get_several_folders_and_only_ones_the_fleet_has() {
    let dir = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("shared")).unwrap();
    let fleet = fleet_with(dir.path(), &[], &[]);
    trust_host(
        &fleet,
        &Keys::default(),
        "minipc",
        &workstation(),
        &[format!("{SIGNING}={}", random())],
        &mut Prompt::answering("types/nowhere\nversions\ntypes/workstation shared\n"),
    )
    .unwrap();
    assert_eq!(
        Machines::load(dir.path())
            .unwrap()
            .get("minipc")
            .unwrap()
            .signs,
        Some(sign::Reach::Folders(vec![
            "types/workstation".into(),
            "shared".into()
        ]))
    );
}

#[test]
fn a_new_recovery_key_opens_every_secret_and_the_old_one_none() {
    let dir = tempfile::TempDir::new().unwrap();
    let laptop = make_key(&dir.path().join("laptop.txt")).unwrap();
    let old = crate::support::name_recovery(dir.path());
    let fleet = fleet_with(
        dir.path(),
        &[("laptop", root_machine(&laptop))],
        &[("wifi.conf.age", b"psk=1", &[&laptop, &old])],
    );
    let new = crate::support::name_recovery(dir.path());
    let repo = crate::fleet::Repo::new(
        dir.path().to_path_buf(),
        "workstation",
        &[],
        &Default::default(),
    )
    .unwrap();
    drop(fleet);
    let fleet = Fleet::of(repo, &crate::support::current_user());
    let file = fleet.etc_dir().join("wifi.conf.age");

    reseal_everything(&fleet, &keys(&old), &mut Prompt::answering("")).unwrap();
    assert!(opens(&new, &file));
    assert!(opens(&laptop, &file));
    assert!(!opens(&old, &file));
    assert!(!dir.path().join("types/workstation/users").exists());
}

#[test]
fn the_probe_signs_only_versions_and_its_ssh_key_goes_to_github() {
    let dir = tempfile::TempDir::new().unwrap();
    let fleet = fleet_with(dir.path(), &[], &[]);
    let (signing, login) = (random(), random());
    let uploads = trust(
        &fleet,
        &Keys::default(),
        "minipc",
        &[format!("{PROBE}={signing}"), format!("{PROBE_SSH}={login}")],
    )
    .unwrap();
    assert_eq!(
        uploads,
        [github::Key {
            title: "minipc/probe".into(),
            public: login,
            kind: github::Use::Deploy,
        }]
    );
    let signers = Machines::load(dir.path()).unwrap().signers().unwrap();
    assert_eq!(signers[0].name, "minipc/probe");
    assert_eq!(signers[0].reach, sign::Reach::Versions);
    let fleet = fleet_with(dir.path(), &[], &[]);
    retire_host(&fleet, "minipc").unwrap();
    assert!(!machines::path(dir.path(), "minipc").exists());
}

#[test]
fn a_shared_secret_root_links_is_for_the_root_list_too() {
    let dir = tempfile::TempDir::new().unwrap();
    let fleet = dir.path();
    let dev = fleet.join("types/server/users/dev");
    fs::create_dir_all(dev.join("home/.secrets")).unwrap();
    crate::support::machine(
        fleet,
        "minipc",
        "type = \"server\"\n[keys]\nage = \"age1root\"\n[users.dev]\nage = \"age1dev\"\n",
    );
    fs::create_dir_all(fleet.join("shared/.secrets")).unwrap();
    fs::write(fleet.join("shared/.secrets/token.age"), "").unwrap();
    symlink(
        "../../../../../../shared/.secrets/token.age",
        dev.join("home/.secrets/token.age"),
    )
    .unwrap();
    let etc = fleet.join("types/server/etc/mothership/probe");
    fs::create_dir_all(&etc).unwrap();
    symlink(
        "../../../../../shared/.secrets/token.age",
        etc.join("github-token.age"),
    )
    .unwrap();
    let machines = Machines::load(fleet).unwrap();
    for link in [
        dev.join("home/.secrets/token.age"),
        etc.join("github-token.age"),
    ] {
        let readers = homes::owners(fleet, &link).unwrap();
        assert_eq!(
            keys_of(machines.recipients_of(&readers)),
            ["age1root", "age1dev"]
        );
    }
}
