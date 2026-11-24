use super::*;
use crate::{
    fleet::{Repo, SYSTEM_TOML},
    secrets::Keys,
    services::tests::toml,
    support,
};

fn fleet(dir: &std::path::Path, root: &age::x25519::Identity) -> (Fleet, age::x25519::Identity) {
    let server = dir.join("types/server");
    let app = server.join("services/app");
    fs::create_dir_all(server.join("etc")).unwrap();
    fs::create_dir_all(&app).unwrap();
    fs::write(server.join(SYSTEM_TOML), "").unwrap();
    support::machine(
        dir,
        "minipc",
        &format!(
            "type = \"server\"\n[keys]\nage = \"{}\"\n",
            root.to_public()
        ),
    );
    fs::write(app.join(SERVICE_TOML), toml("1.0.0")).unwrap();
    fs::write(app.join("compose.yaml"), "services: {app: {}}\n").unwrap();
    let recovery = support::name_recovery(dir);
    let repo = Repo::new(dir.to_path_buf(), "server", &[], &Default::default()).unwrap();
    (Fleet::of(repo, &support::current_user()), recovery)
}

fn opens(key: &age::x25519::Identity, path: &std::path::Path) -> String {
    let mut keys = Keys::default();
    keys.push(key.clone());
    String::from_utf8(keys.decrypt(path).unwrap()).unwrap()
}

#[test]
fn a_secret_is_for_the_types_root_list_and_the_recovery_key() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = age::x25519::Identity::generate();
    let (fleet, recovery) = fleet(dir.path(), &root);
    let wanted = Wanted::parse("app", "db-password", "server").unwrap();
    set(&fleet, &wanted, &mut Prompt::answering("hunter2\n")).unwrap();
    let path = dir
        .path()
        .join("types/server/services/app/secrets/db-password.age");
    assert_eq!(opens(&root, &path), "hunter2");
    assert_eq!(opens(&recovery, &path), "hunter2");
}

#[test]
fn a_secret_names_a_service_the_fleet_has() {
    let dir = tempfile::TempDir::new().unwrap();
    let (fleet, _) = fleet(dir.path(), &age::x25519::Identity::generate());
    let wanted = Wanted::parse("server/web", "key", "workstation").unwrap();
    assert_eq!(wanted.shown(), "server/web");
    let err = set(&fleet, &wanted, &mut Prompt::answering("x\n")).unwrap_err();
    assert!(err.to_string().contains("no service server/web"), "{err}");
    assert!(Wanted::parse("app", "../key", "server").is_err());
    assert!(Wanted::parse("../app", "key", "server").is_err());
}
