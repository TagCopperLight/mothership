use super::*;
use crate::fleet::homes::Home;

const LAPTOP: &str = r#"
type = "workstation"
signs = "everything"

[keys]
signing = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEZDnIj3gcD4o1XxMPApraR3MutSyGl0aHC+FiaaUlm5"
age = "age1laptop"
"#;

const MINIPC: &str = r#"
type = "server"
signs = ["types/server"]

[keys]
signing = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIM+ekU0xZNU5xmDldz5pWKFHJ0RD4iFhYSDw48k6qNnH"
age = "age1minipc"

[users.dev]
age = "age1dev"
"#;

const PROBED: &str = r#"
type = "server"

[keys]
age = "age1probed"
probe = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBiaU3yvKl1XfZP1NPwoqvUpk/NKEmD0N2A069AmOCrB"
"#;

fn machines(files: &[(&str, &str)]) -> Result<Machines> {
    Machines::parse(
        files
            .iter()
            .map(|(name, text)| (Path::new(MACHINES).join(name), text.to_string()))
            .collect(),
    )
}

fn names(list: Vec<Recipient>) -> Vec<String> {
    list.into_iter().map(|r| r.name).collect()
}

#[test]
fn a_machine_opens_its_own_secrets_and_those_of_every_folder_it_signs() {
    let all = machines(&[
        ("laptop.toml", LAPTOP),
        ("minipc.toml", MINIPC),
        ("probed.toml", PROBED),
    ])
    .unwrap();
    let dev = Reader::Home(Home {
        type_name: "server".into(),
        user: "dev".into(),
    });
    assert_eq!(
        names(all.recipients(&Reader::Root("server".into()))),
        ["laptop", "minipc", "probed"]
    );
    let keys: Vec<String> = all.recipients(&dev).into_iter().map(|r| r.key).collect();
    assert_eq!(keys, ["age1laptop", "age1dev", "age1minipc"]);
    assert_eq!(
        names(all.recipients(&Reader::Root("workstation".into()))),
        ["laptop"]
    );
}

#[test]
fn each_signing_key_becomes_a_signer_with_its_reach() {
    let all = machines(&[("minipc.toml", MINIPC), ("probed.toml", PROBED)]).unwrap();
    let signers: Vec<(String, Reach)> = all
        .signers()
        .unwrap()
        .into_iter()
        .map(|s| (s.name, s.reach))
        .collect();
    assert_eq!(
        signers,
        [
            (
                "minipc".to_owned(),
                Reach::Folders(vec!["types/server".into()])
            ),
            ("probed/probe".to_owned(), Reach::Versions),
        ]
    );
}

#[test]
fn signs_is_everything_or_folders_and_comes_with_a_signing_key() {
    let with = |signs: &str| {
        let text = MINIPC.replace(r#"signs = ["types/server"]"#, signs);
        machines(&[("minipc.toml", &text)])
    };
    assert!(with(r#"signs = "everything""#).is_ok());
    for refused in [
        r#"signs = "all""#,
        "signs = []",
        r#"signs = ["versions"]"#,
        r#"signs = ["../etc"]"#,
        "",
    ] {
        assert!(with(refused).is_err(), "{refused}");
    }
}

#[test]
fn a_machine_file_is_named_after_its_machine() {
    for refused in ["minipc.txt", "Mini PC.toml", "nested/minipc.toml"] {
        assert!(machines(&[(refused, PROBED)]).is_err(), "{refused}");
    }
}

#[test]
fn a_written_machine_reads_back_the_same() {
    let dir = tempfile::TempDir::new().unwrap();
    let all = machines(&[("minipc.toml", MINIPC)]).unwrap();
    all.write(dir.path(), "minipc").unwrap();
    assert_eq!(Machines::load(dir.path()).unwrap(), all);
}

#[test]
fn a_type_or_user_that_is_no_plain_name_is_refused() {
    let escaping = "type = \"db-server/../workstation\"\n";
    let error = machines(&[("db.toml", escaping)]).unwrap_err();
    assert_eq!(
        format!("{error:#}"),
        "machines/db.toml's type: \"db-server/../workstation\" isn't a plain name: use one folder name, without / or .."
    );
    let user = "type = \"server\"\n[users.\"../dev\"]\n";
    assert!(machines(&[("db.toml", user)]).is_err());
}
