use super::*;
use ssh_key::{Algorithm, LineEnding, PrivateKey, rand_core::OsRng};
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

struct Repo {
    _dir: TempDir,
    work: PathBuf,
    keys: PathBuf,
}

impl Repo {
    fn new() -> Repo {
        let dir = TempDir::new().unwrap();
        let work = dir.path().join("fleet");
        let keys = dir.path().join("keys");
        fs::create_dir_all(&keys).unwrap();
        fs::create_dir_all(&work).unwrap();
        let repo = Repo {
            _dir: dir,
            work,
            keys,
        };
        repo.git(&["init", "-q", "-b", "main"]);
        repo
    }

    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.work)
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
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    fn key(&self, name: &str) -> (PrivateKey, PathBuf) {
        let key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap();
        let path = self.keys.join(name);
        fs::write(&path, key.to_openssh(LineEnding::LF).unwrap().as_bytes()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        (key, path)
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.work.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn commit(&self, message: &str, by: Option<&Path>) -> String {
        self.git(&["add", "-A"]);
        let signing;
        let mut args = vec!["commit", "-q", "--allow-empty", "-m", message];
        if let Some(key) = by {
            signing = format!("user.signingkey={}", key.display());
            args = [
                &["-c", "gpg.format=ssh", "-c", &signing][..],
                &args[..],
                &["-S"],
            ]
            .concat();
        }
        self.git(&args);
        self.git(&["rev-parse", "HEAD"])
    }

    fn check(&self, from: &str) -> Checked {
        check(
            &Git::at(self.work.join(".git")),
            from,
            &self.git(&["rev-parse", "HEAD"]),
        )
        .unwrap()
    }
}

fn line(name: &str, key: &PrivateKey, scope: &str) -> String {
    format!("# {name}\n{} {scope}\n", sign::public(key).unwrap())
}

#[test]
fn a_chain_signed_by_listed_keys_passes_and_stops_at_the_first_unsigned_commit() {
    let repo = Repo::new();
    let (laptop, laptop_file) = repo.key("laptop");
    repo.write(SIGNERS, &line("laptop", &laptop, ""));
    let anchor = repo.commit("Signers", None);
    repo.write("types/a/system.toml", "packages = []\n");
    let signed = repo.commit("Signed", Some(&laptop_file));
    repo.write("types/a/system.toml", "packages = [\"evil\"]\n");
    let unsigned = repo.commit("Unsigned", None);
    repo.commit("After", Some(&laptop_file));

    let checked = repo.check(&anchor);
    assert_eq!(checked.commit, signed);
    let refused = checked.refused.unwrap();
    assert_eq!(refused.commit, unsigned);
    assert_eq!(refused.subject, "Unsigned");
    assert_eq!(refused.why, "nobody signed it");
}

#[test]
fn a_commit_can_not_sign_itself_in() {
    let repo = Repo::new();
    let (laptop, _) = repo.key("laptop");
    let (rogue, rogue_file) = repo.key("rogue");
    repo.write(SIGNERS, &line("laptop", &laptop, ""));
    let anchor = repo.commit("Signers", None);
    repo.write(
        SIGNERS,
        &(line("laptop", &laptop, "") + "\n" + &line("rogue", &rogue, "")),
    );
    repo.commit("Let me in", Some(&rogue_file));

    let checked = repo.check(&anchor);
    assert_eq!(checked.commit, anchor);
    assert_eq!(
        checked.refused.unwrap().why,
        "a key the fleet doesn't list signed it"
    );
}

#[test]
fn a_scoped_signer_stays_in_its_type_and_recovery_signs_anything() {
    let repo = Repo::new();
    let (minipc, minipc_file) = repo.key("minipc");
    let age = age::x25519::Identity::generate();
    let recovery = sign::recovery_key(&age);
    let recovery_file = repo.keys.join("recovery");
    fs::write(
        &recovery_file,
        recovery.to_openssh(LineEnding::LF).unwrap().as_bytes(),
    )
    .unwrap();
    fs::set_permissions(&recovery_file, fs::Permissions::from_mode(0o600)).unwrap();
    repo.write(SIGNERS, &line("minipc", &minipc, "types/minipc"));
    repo.write(
        MANIFEST,
        &format!(
            "[recovery]\nage = \"{}\"\nsigning = \"{}\"\n",
            age.to_public(),
            sign::public(&recovery).unwrap()
        ),
    );
    let anchor = repo.commit("Signers", None);
    repo.write("types/minipc/system.toml", "packages = []\n");
    let own = repo.commit("Own type", Some(&minipc_file));
    repo.write("shared/dot_gitconfig", "[user]\n");
    repo.commit("Shared", Some(&minipc_file));

    let checked = repo.check(&anchor);
    assert_eq!(checked.commit, own);
    assert_eq!(
        checked.refused.unwrap().why,
        "minipc can't sign a change to shared/dot_gitconfig"
    );

    repo.git(&["reset", "-q", "--hard", &own]);
    repo.write("shared/dot_gitconfig", "[user]\n");
    let recovered = repo.commit("Shared, by recovery", Some(&recovery_file));
    let checked = repo.check(&anchor);
    assert!(checked.refused.is_none());
    assert_eq!(checked.commit, recovered);
}

#[test]
fn a_main_that_left_the_checked_commit_stops_the_check() {
    let repo = Repo::new();
    let anchor = repo.commit("First", None);
    repo.git(&["checkout", "-q", "--orphan", "other"]);
    repo.commit("Rewritten", None);
    let error = check(
        &Git::at(repo.work.join(".git")),
        &anchor,
        &repo.git(&["rev-parse", "HEAD"]),
    )
    .err()
    .unwrap();
    assert!(error.to_string().starts_with("main doesn't follow"));
}

#[test]
fn a_main_and_an_origin_that_split_stop_the_check() {
    let repo = Repo::new();
    let first = repo.commit("First", None);
    let ours = repo.commit("Ours", None);
    repo.git(&["checkout", "-q", "-b", "theirs", &first]);
    let theirs = repo.commit("Theirs", None);
    let git = Git::at(repo.work.join(".git"));
    git.out(&["update-ref", MAIN, &ours]).unwrap();
    git.out(&["update-ref", ORIGIN, &first]).unwrap();
    assert_eq!(tips(&git).unwrap(), (ours.clone(), ours.clone()));
    git.out(&["update-ref", ORIGIN, &theirs]).unwrap();
    let error = tips(&git).unwrap_err().to_string();
    assert!(
        error.starts_with("main and origin/main have split"),
        "{error}"
    );
}

const OLD: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
const NEW: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";

fn pinned(version: &str, digest: &str) -> String {
    format!("[images.db]\nimage = \"postgres\"\nversion = \"{version}\"\ndigest = \"{digest}\"\n")
}

#[test]
fn a_versions_signer_changes_only_versions_and_digests_within_their_major() {
    let repo = Repo::new();
    let (laptop, _) = repo.key("laptop");
    let (probe, probe_file) = repo.key("probe");
    repo.write(
        SIGNERS,
        &(line("laptop", &laptop, "") + "\n" + &line("probe", &probe, "versions")),
    );
    let service = "types/server/services/db/service.toml";
    repo.write(service, &pinned("16.4.3", OLD));
    repo.write("types/server/services/db/compose.yaml", "services: {}\n");
    let anchor = repo.commit("Signers", None);
    repo.write(service, &pinned("16.5.0", NEW));
    let update = repo.commit("Update", Some(&probe_file));
    repo.write(service, &pinned("16.5.0", OLD));
    let digest = repo.commit("Same version, new digest", Some(&probe_file));
    let checked = repo.check(&anchor);
    assert!(checked.refused.is_none());
    assert_eq!(checked.commit, digest);
    assert!(matches!(checked.steps[0].reach, Reach::Versions));
    assert_eq!(checked.steps[0].commit, update);

    let refused = |text: &str, path: &str| {
        repo.git(&["reset", "-q", "--hard", &digest]);
        repo.write(path, text);
        repo.commit("Probe", Some(&probe_file));
        repo.check(&digest).refused.unwrap().why
    };
    assert_eq!(
        refused(&pinned("17.0.0", NEW), service),
        format!(
            "probe can't sign this change to {service}: images.db: 17.0.0 leaves major version 16"
        )
    );
    assert_eq!(
        refused(&pinned("16.4.0", NEW), service),
        format!(
            "probe can't sign this change to {service}: images.db: 16.4.0 is older than 16.5.0"
        )
    );
    assert_eq!(
        refused(
            &pinned("16.5.0", OLD).replace("postgres", "evil/postgres"),
            service
        ),
        format!(
            "probe can't sign this change to {service}: it changes images.db beyond its version and digest"
        )
    );
    assert_eq!(
        refused(
            "services: {evil: {}}\n",
            "types/server/services/db/compose.yaml"
        ),
        "probe can't sign a change to types/server/services/db/compose.yaml"
    );
    assert_eq!(
        refused(
            &pinned("1.0.0", NEW),
            "types/server/services/new/service.toml"
        ),
        "probe can't add or remove types/server/services/new/service.toml"
    );
    repo.git(&["reset", "-q", "--hard", &digest]);
    repo.write(SIGNERS, &line("laptop", &laptop, ""));
    repo.commit("Drop itself", Some(&probe_file));
    assert_eq!(
        repo.check(&digest).refused.unwrap().why,
        "probe can't change probe's line in signers.txt"
    );
}

#[test]
fn pins_a_versions_signer_sets_wait_until_a_full_signer_changes_them() {
    let mut unconfirmed = Unconfirmed::default();
    let pin = |version: &str| container::Pin {
        version: version.into(),
        digest: NEW.into(),
    };
    unconfirmed.note("server/db", vec![("db".into(), pin("16.5.0"))], true);
    assert!(unconfirmed.waits("server/db", "db", &pin("16.5.0")));
    assert!(!unconfirmed.waits("server/db", "db", &pin("16.4.0")));
    unconfirmed.note("server/db", vec![("db".into(), pin("16.6.0"))], false);
    assert_eq!(unconfirmed, Unconfirmed::default());
}
