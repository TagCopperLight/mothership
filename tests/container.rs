use super::*;
use crate::secrets::{self, Recipient};

pub const DIGEST: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";

pub fn toml(version: &str) -> String {
    format!(
        "[images.app]\nimage = \"ghcr.io/me/app\"\nversion = \"{version}\"\ndigest = \"{DIGEST}\"\n"
    )
}

#[test]
fn a_service_pins_images_with_versions_and_digests() {
    let images = parse(&toml("1.2.3")).unwrap();
    assert_eq!(
        images["app"].reference(&images["app"].pin()),
        format!("ghcr.io/me/app:1.2.3@{DIGEST}")
    );
    let err = |text: &str| format!("{:#}", parse(text).unwrap_err());
    assert!(err(&toml("latest")).contains("the fleet runs only images with versions"));
    assert!(err(&toml("1").replace(DIGEST, "sha256:12")).contains("isn't a digest"));
    assert!(err(&toml("1").replace("ghcr.io/me/app", "app:1")).contains("without a tag"));
    assert!(err("").contains("it pins no image"));
    parse(&toml("1").replace("ghcr.io/me/app", "localhost:5000/app")).unwrap();
}

fn service(dir: &Path) -> Service {
    fs::create_dir_all(dir.join(SECRETS)).unwrap();
    fs::write(dir.join(SERVICE_TOML), toml("1.0.0")).unwrap();
    fs::write(dir.join(COMPOSE), "services:\n  app: {}\n").unwrap();
    load(dir.parent().unwrap().parent().unwrap())
        .unwrap()
        .remove(0)
}

#[test]
fn rendering_decrypts_secrets_and_fills_templates() {
    let dir = tempfile::tempdir().unwrap();
    let svc = dir.path().join("services/app");
    let key = age::x25519::Identity::generate();
    let list = [Recipient {
        name: "root".into(),
        key: secrets::public(&key),
    }];
    fs::create_dir_all(svc.join(SECRETS)).unwrap();
    fs::write(
        svc.join("secrets/token.age"),
        secrets::encrypt_for(&list, b"hunter2\n").unwrap(),
    )
    .unwrap();
    fs::write(
        svc.join("app.conf.tmpl"),
        "token = {{ secret \"token\" }}\n",
    )
    .unwrap();
    let service = service(&svc);
    let mut keys = Keys::default();
    keys.push(key);
    let rendered = service.render(&keys).unwrap();
    assert_eq!(rendered.files[Path::new("app.conf")], b"token = hunter2\n");
    assert!(rendered.files.contains_key(Path::new(COMPOSE)));
    assert_eq!(rendered.secrets["token"], b"hunter2\n");
    fs::write(svc.join("bad.tmpl"), "{{ secret \"nope\" }}").unwrap();
    let err = format!("{:#}", service.render(&keys).unwrap_err());
    assert!(err.contains("the service has no secret nope"), "{err}");
}

#[test]
fn a_service_holds_no_links() {
    let dir = tempfile::tempdir().unwrap();
    let svc = dir.path().join("services/app");
    let service = service(&svc);
    std::os::unix::fs::symlink("/etc/shadow", svc.join("shadow")).unwrap();
    let err = format!("{:#}", service.render(&Keys::default()).unwrap_err());
    assert!(err.contains("is a link"), "{err}");
}

#[test]
fn a_snapshot_names_paths_inside_the_data_folder() {
    let with = |parts: &str| format!("snapshot = [{parts}]\n{}", toml("1.2.3"));
    assert_eq!(
        parse_toml(&with("\"db\", \"a/b\"")).unwrap().snapshot,
        [PathBuf::from("db"), PathBuf::from("a/b")]
    );
    for bad in ["\"../etc\"", "\"/etc\"", "\"\"", "\"./db\""] {
        let err = format!("{:#}", parse_toml(&with(bad)).unwrap_err());
        assert!(err.contains("isn't a path inside"), "{bad}: {err}");
    }
}

#[test]
fn a_versions_signer_leaves_the_snapshot_alone() {
    let before = toml("1.2.3");
    let after = format!("snapshot = [\"db\"]\n{}", toml("1.2.4"));
    let err = format!("{:#}", versions_only(&before, &after).unwrap_err());
    assert!(err.contains("what the snapshot covers"), "{err}");
}
