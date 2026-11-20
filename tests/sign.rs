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

fn signer(name: &str, key: &PrivateKey, scope: Option<&str>) -> Signer {
    Signer {
        name: name.into(),
        key: key.public_key().clone(),
        scope: scope.map(PathBuf::from),
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
        signer("stranger", &stranger, None),
        signer("laptop", &laptop, None),
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
fn reads_a_scope_after_the_key() {
    let (laptop, minipc) = (random(), random());
    let text = format!(
        "# laptop\n{}\n\n# minipc\n{} types/minipc\n",
        public(&laptop).unwrap(),
        public(&minipc).unwrap()
    );
    assert_eq!(
        parse_signers(&text).unwrap(),
        [
            signer("laptop", &laptop, None),
            signer("minipc", &minipc, Some("types/minipc"))
        ]
    );
    let extra = format!("# laptop\n{} types/a types/b\n", public(&laptop).unwrap());
    assert!(parse_signers(&extra).is_err());
}

#[test]
fn a_scoped_signer_changes_its_type_and_hands_out_no_more_than_it_has() {
    let (laptop, minipc, other) = (random(), random(), random());
    let workstation = signer("laptop", &laptop, None);
    let server = signer("minipc", &minipc, Some("types/minipc"));
    let paths = |list: &[&str]| list.iter().map(PathBuf::from).collect::<Vec<_>>();
    let before = [workstation.clone(), server.clone()];

    assert!(
        allowed(
            &server,
            &paths(&["types/minipc/system.toml"]),
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

    let sibling = [
        workstation.clone(),
        server.clone(),
        signer("minipc2", &other, Some("types/minipc")),
    ];
    assert!(allowed(&server, &paths(&[SIGNERS]), &before, &sibling).is_ok());
    let wider = [
        workstation.clone(),
        server.clone(),
        signer("rogue", &other, None),
    ];
    assert_eq!(
        allowed(&server, &paths(&[SIGNERS]), &before, &wider)
            .unwrap_err()
            .to_string(),
        "minipc can't change rogue's line in signers.txt"
    );
    let dropped = [server.clone()];
    assert!(allowed(&server, &paths(&[SIGNERS]), &before, &dropped).is_err());
    assert!(allowed(&workstation, &paths(&[SIGNERS]), &before, &wider).is_ok());
}
