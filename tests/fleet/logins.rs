use super::*;
use std::fs;

fn key(byte: u8) -> String {
    let keypair = ssh_key::private::Ed25519Keypair::from_seed(&[byte; 32]);
    ssh_key::PrivateKey::from(keypair)
        .public_key()
        .to_openssh()
        .unwrap()
}

fn fleet(users: &[(&str, &str, &str, Option<&str>)]) -> (tempfile::TempDir, Machines) {
    let dir = tempfile::TempDir::new().unwrap();
    let mut texts: std::collections::BTreeMap<&str, String> = Default::default();
    for (machine, type_name, user, key) in users {
        let user_dir = dir
            .path()
            .join("types")
            .join(type_name)
            .join(USERS)
            .join(user);
        fs::create_dir_all(&user_dir).unwrap();
        let text = texts
            .entry(machine)
            .or_insert(format!("type = \"{type_name}\"\n"));
        if let Some(key) = key {
            text.push_str(&format!("[users.{user}]\nssh = \"{key}\"\n"));
        }
    }
    let files = texts
        .into_iter()
        .map(|(machine, text)| (Path::new("machines").join(format!("{machine}.toml")), text))
        .collect();
    (dir, Machines::parse(files).unwrap())
}

fn from(fleet: &(tempfile::TempDir, Machines), entries: &[&str]) -> Result<Option<Vec<String>>> {
    let entries: Vec<String> = entries.iter().map(|e| e.to_string()).collect();
    resolve(fleet.0.path(), &fleet.1, Path::new("user.toml"), &entries)
}

#[test]
fn a_bare_type_lets_in_all_its_users_and_a_type_and_user_only_that_one() {
    let (laptop_tag, laptop_dev, minipc_dev) = (key(1), key(2), key(3));
    let fleet = fleet(&[
        (
            "laptop",
            "workstation",
            "tag",
            Some(&format!("{laptop_tag} laptop/tag")),
        ),
        ("laptop", "workstation", "dev", Some(&laptop_dev)),
        ("minipc", "minipc", "dev", Some(&minipc_dev)),
        ("minipc", "minipc", "tag", None),
    ]);
    assert_eq!(from(&fleet, &[]).unwrap(), None);
    assert_eq!(
        from(&fleet, &["workstation", "minipc/dev", "workstation/tag"]).unwrap(),
        Some(vec![
            format!("{laptop_dev} laptop/dev"),
            format!("{laptop_tag} laptop/tag"),
            format!("{minipc_dev} minipc/dev"),
        ])
    );
}

#[test]
fn a_name_the_fleet_lacks_or_no_key_at_all_stops_the_fleet() {
    let fleet = fleet(&[("minipc", "minipc", "tag", None)]);
    let error = |entries: &[&str]| from(&fleet, entries).unwrap_err().to_string();
    assert!(error(&["workstaton"]).ends_with("the fleet has no type workstaton"));
    assert!(error(&["minipc/me"]).ends_with("minipc has no user me"));
    assert!(error(&["minipc/../minipc/tag"]).ends_with("minipc has no user ../minipc/tag"));
    assert!(error(&["minipc"]).contains("finds no SSH key"));
}
