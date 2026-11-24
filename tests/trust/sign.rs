use super::*;
use std::{fs, os::unix::fs::PermissionsExt, process::Command};
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=Me", "-c", "user.email=me@example.com"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

fn key_file(dir: &Path, key: &PrivateKey) -> PathBuf {
    let path = dir.join("signing");
    fs::write(&path, key.to_openssh(LineEnding::LF).unwrap().as_bytes()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    path
}

fn signer(name: &str, key: &PrivateKey, folders: &[&str]) -> Signer {
    Signer {
        name: name.into(),
        key: key.public_key().clone(),
        reach: Reach::parse(folders).unwrap(),
    }
}

fn random() -> PrivateKey {
    PrivateKey::random(&mut OsRng, ssh_key::Algorithm::Ed25519).unwrap()
}

#[test]
fn finds_who_signed_a_commit_git_signed() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    let (laptop, stranger) = (random(), random());
    let path = key_file(dir.path(), &laptop);
    git(dir.path(), &["init", "-q", repo.to_str().unwrap()]);
    let signing = format!("user.signingkey={}", path.display());
    git(
        &repo,
        &[
            "-c",
            "gpg.format=ssh",
            "-c",
            &signing,
            "commit",
            "-q",
            "-S",
            "--allow-empty",
            "-m",
            "Signed\n\ngpgsig in the message",
        ],
    );
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "Unsigned"]);
    let signers = [
        signer("stranger", &stranger, &[]),
        signer("laptop", &laptop, &[]),
    ];

    let signed = Commit::parse(&git(&repo, &["cat-file", "commit", "HEAD~"])).unwrap();
    assert_eq!(signed.signer(&signers).unwrap().name, "laptop");
    assert!(signed.signer(&signers[..1]).is_none());
    assert!(!String::from_utf8_lossy(&signed.payload).contains("BEGIN SSH SIGNATURE"));

    let unsigned = Commit::parse(&git(&repo, &["cat-file", "commit", "HEAD"])).unwrap();
    assert!(unsigned.signature.is_none());
    assert!(unsigned.signer(&signers).is_none());

    let mut forged = signed;
    forged.payload = String::from_utf8(forged.payload)
        .unwrap()
        .replace("Signed", "Forged")
        .into_bytes();
    assert!(forged.signer(&signers).is_none());
}

#[test]
fn ssh_keygen_accepts_what_it_signs() {
    let dir = TempDir::new().unwrap();
    let key = random();
    let sig = dir.path().join("payload.sig");
    fs::write(&sig, sign(&key, b"tree abc\n\nmessage\n").unwrap()).unwrap();
    let allowed = dir.path().join("allowed");
    fs::write(&allowed, format!("me {}\n", public(&key).unwrap())).unwrap();
    let mut check = Command::new("ssh-keygen")
        .args(["-Y", "verify", "-n", "git", "-I", "me", "-f"])
        .arg(&allowed)
        .arg("-s")
        .arg(&sig)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    use std::io::Write;
    check
        .stdin
        .take()
        .unwrap()
        .write_all(b"tree abc\n\nmessage\n")
        .unwrap();
    assert!(check.wait().unwrap().success());
}

#[test]
fn the_recovery_signer_comes_back_from_the_age_key_alone() {
    let age = x25519::Identity::generate();
    let again: x25519::Identity = age.to_string().expose_secret().parse().unwrap();
    assert_eq!(
        public(&recovery_key(&age)).unwrap(),
        public(&recovery_key(&again)).unwrap()
    );
    let other = x25519::Identity::generate();
    assert_ne!(
        public(&recovery_key(&age)).unwrap(),
        public(&recovery_key(&other)).unwrap()
    );
}

#[test]
fn a_key_listed_twice_or_the_recovery_key_is_refused() {
    let (laptop, minipc) = (random(), random());
    let list = [
        signer("laptop", &laptop, &[]),
        signer("minipc", &minipc, &["types/minipc"]),
    ];
    assert!(distinct(&list, None).is_ok());
    let trap = signer("trap", &laptop, &["types/minipc"]);
    let twice = [list[0].clone(), list[1].clone(), trap];
    assert_eq!(
        distinct(&twice, None).unwrap_err().to_string(),
        "machines/ lists one signing key twice, as laptop and trap"
    );
    let recovery = Signer::recovery(&public(&minipc).unwrap()).unwrap();
    assert_eq!(
        distinct(&list, Some(&recovery)).unwrap_err().to_string(),
        "machines/ lists the recovery key, as minipc"
    );
}

fn machine(type_name: &str, signs: &str, key: &PrivateKey) -> String {
    format!(
        "type = \"{type_name}\"\nsigns = {signs}\n[keys]\nsigning = \"{}\"\n",
        public(key).unwrap()
    )
}

fn machines(files: &[(&str, &str)]) -> Machines {
    Machines::parse(
        files
            .iter()
            .map(|(name, text)| {
                (
                    PathBuf::from(format!("machines/{name}.toml")),
                    text.to_string(),
                )
            })
            .collect(),
    )
    .unwrap()
}

#[test]
fn a_scoped_signer_changes_its_type_and_hands_out_no_more_than_it_has() {
    let (laptop, minipc, other) = (random(), random(), random());
    let workstation = signer("laptop", &laptop, &[]);
    let server = signer("minipc", &minipc, &["types/minipc", "shared/minipc"]);
    let paths = |list: &[&str]| list.iter().map(PathBuf::from).collect::<Vec<_>>();
    let laptop_file = machine("workstation", "\"everything\"", &laptop);
    let minipc_file = machine("minipc", r#"["types/minipc", "shared/minipc"]"#, &minipc);
    let before = machines(&[("laptop", &laptop_file), ("minipc", &minipc_file)]);

    assert!(
        allowed(
            &server,
            &paths(&["types/minipc/system.toml", "shared/minipc/token.age"]),
            &before,
            &before
        )
        .is_ok()
    );
    let outside = allowed(&server, &paths(&["shared/dot_gitconfig"]), &before, &before);
    assert_eq!(
        outside.unwrap_err().to_string(),
        "minipc can't sign a change to shared/dot_gitconfig"
    );
    assert!(
        allowed(
            &workstation,
            &paths(&["shared/dot_gitconfig", crate::fleet::MANIFEST]),
            &before,
            &before
        )
        .is_ok()
    );

    let with = |name: &str, text: &str| {
        machines(&[
            ("laptop", &laptop_file),
            ("minipc", &minipc_file),
            (name, text),
        ])
    };
    let change = |signer: &Signer, name: &str, before: &Machines, after: &Machines| {
        allowed(
            signer,
            &paths(&[&format!("machines/{name}.toml")]),
            before,
            after,
        )
        .map_err(|e| e.to_string())
    };
    let sibling = with(
        "minipc2",
        &machine("minipc", r#"["types/minipc/users"]"#, &other),
    );
    assert!(change(&server, "minipc2", &before, &sibling).is_ok());
    let wider = with("rogue", &machine("minipc", "\"everything\"", &other));
    assert_eq!(
        change(&server, "rogue", &before, &wider).unwrap_err(),
        "minipc can't change rogue, which signs more than it"
    );
    let elsewhere = with(
        "rogue",
        &machine("workstation", r#"["types/minipc"]"#, &other),
    );
    assert_eq!(
        change(&server, "rogue", &before, &elsewhere).unwrap_err(),
        "minipc can't change rogue, a machine of workstation"
    );
    let dropped = machines(&[("minipc", &minipc_file)]);
    assert!(change(&server, "laptop", &before, &dropped).is_err());
    let optioned = machines(&[
        ("laptop", &laptop_file),
        (
            "minipc",
            &minipc_file.replace("signs", "options = [\"gpu\"]\nsigns"),
        ),
    ]);
    assert!(change(&server, "minipc", &before, &optioned).is_ok());
    assert!(change(&workstation, "rogue", &before, &wider).is_ok());
    let probed = with(
        "probed",
        &format!(
            "type = \"minipc\"\n[keys]\nprobe = \"{}\"\n",
            public(&other).unwrap()
        ),
    );
    assert!(change(&server, "probed", &before, &probed).is_err());
}
