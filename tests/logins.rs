use super::*;
use std::fs;

fn key(byte: u8) -> String {
    let keypair = ssh_key::private::Ed25519Keypair::from_seed(&[byte; 32]);
    ssh_key::PrivateKey::from(keypair)
        .public_key()
        .to_openssh()
        .unwrap()
}

fn fleet(users: &[(&str, &str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    for (type_name, user, keys) in users {
        let user_dir = dir
            .path()
            .join("types")
            .join(type_name)
            .join(USERS)
            .join(user);
        fs::create_dir_all(&user_dir).unwrap();
        fs::write(user_dir.join(ssh::KEYS), keys).unwrap();
    }
    dir
}

fn from(dir: &Path, entries: &[&str]) -> Result<Option<Vec<String>>> {
    let entries: Vec<String> = entries.iter().map(|e| e.to_string()).collect();
    resolve(dir, Path::new("user.toml"), &entries)
}

#[test]
fn a_bare_type_lets_in_all_its_users_and_a_type_and_user_only_that_one() {
    let (laptop_tag, laptop_dev, minipc_dev) = (key(1), key(2), key(3));
    let dir = fleet(&[
        (
            "workstation",
            "tag",
            &format!("# laptop\n{laptop_tag} laptop/tag\n"),
        ),
        ("workstation", "dev", &format!("# laptop\n{laptop_dev}\n")),
        ("minipc", "dev", &format!("# minipc\n{minipc_dev}\n")),
        ("minipc", "tag", ""),
    ]);
    assert_eq!(from(dir.path(), &[]).unwrap(), None);
    assert_eq!(
        from(
            dir.path(),
            &["workstation", "minipc/dev", "workstation/tag"]
        )
        .unwrap(),
        Some(vec![
            format!("{laptop_dev} laptop/dev"),
            format!("{laptop_tag} laptop/tag"),
            format!("{minipc_dev} minipc/dev"),
        ])
    );
}

#[test]
fn a_name_the_fleet_lacks_or_no_key_at_all_stops_the_fleet() {
    let dir = fleet(&[("minipc", "tag", "")]);
    let error = |entries: &[&str]| from(dir.path(), entries).unwrap_err().to_string();
    assert!(error(&["workstaton"]).ends_with("the fleet has no type workstaton"));
    assert!(error(&["minipc/me"]).ends_with("minipc has no user me"));
    assert!(error(&["minipc/../minipc/tag"]).ends_with("minipc has no user ../minipc/tag"));
    assert!(error(&["minipc"]).contains("finds no SSH key"));
}
