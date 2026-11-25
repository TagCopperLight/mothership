use super::*;
use crate::fleet::MANIFEST;
use ssh_key::{Algorithm, LineEnding, rand_core::OsRng};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;

pub struct Repo {
    _dir: TempDir,
    pub work: PathBuf,
    keys: PathBuf,
}

impl Repo {
    pub fn new() -> Repo {
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

    pub fn git(&self, args: &[&str]) -> String {
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

    pub fn key(&self, name: &str) -> (PrivateKey, PathBuf) {
        let key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap();
        let path = self.keys.join(name);
        fs::write(&path, key.to_openssh(LineEnding::LF).unwrap().as_bytes()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        (key, path)
    }

    pub fn write(&self, rel: &str, text: &str) {
        let path = self.work.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    pub fn commit(&self, message: &str, by: Option<&Path>) -> String {
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

impl Repo {
    pub fn machine(&self, name: &str, key: &PrivateKey, signs: &str) {
        let public = sign::public(key).unwrap();
        let text = match signs {
            "versions" => format!("type = \"server\"\n[keys]\nprobe = \"{public}\"\n"),
            signs => {
                format!("type = \"{name}\"\nsigns = {signs}\n[keys]\nsigning = \"{public}\"\n")
            }
        };
        self.write(&format!("machines/{name}.toml"), &text);
    }
}

pub const EVERYTHING: &str = "\"everything\"";

#[test]
fn a_chain_signed_by_listed_keys_passes_and_stops_at_the_first_unsigned_commit() {
    let repo = Repo::new();
    let (laptop, laptop_file) = repo.key("laptop");
    repo.machine("laptop", &laptop, EVERYTHING);
    let anchor = repo.commit("Signers", None);
    repo.write("types/a/system.toml", "packages = []\n");
    let signed = repo.commit("Signed", Some(&laptop_file));
    repo.write("types/a/system.toml", "packages = [\"evil\"]\n");
    let unsigned = repo.commit("Unsigned", None);
    repo.commit("Unsigned too", None);

    let checked = repo.check(&anchor);
    assert_eq!(checked.commit, signed);
    let refused = checked.refused.unwrap();
    assert_eq!(refused.commit, unsigned);
    assert_eq!(refused.subject, "Unsigned");
    assert_eq!(refused.why, "nobody signed it");
}

#[test]
fn a_full_signer_vouches_for_the_unsigned_commits_before_it() {
    let repo = Repo::new();
    let (laptop, laptop_file) = repo.key("laptop");
    repo.machine("laptop", &laptop, EVERYTHING);
    let anchor = repo.commit("Signers", None);
    repo.write("types/a/system.toml", "packages = [\"zeta\"]\n");
    repo.commit("Unsigned", None);
    let vouched = repo.commit("Vouch", Some(&laptop_file));
    repo.write("types/a/system.toml", "packages = []\n");
    let after = repo.commit("After", Some(&laptop_file));

    let checked = repo.check(&anchor);
    assert!(checked.refused.is_none());
    assert_eq!(checked.commit, after);
    assert_eq!(checked.steps[0].parent, anchor);
    assert_eq!(checked.steps[0].commit, vouched);
    assert_eq!(checked.steps[0].reach, Reach::Everything);

    repo.commit("Unsigned again", None);
    let checked = repo.check(&anchor);
    assert_eq!(checked.commit, after);
    assert_eq!(checked.refused.unwrap().subject, "Unsigned again");
}

#[test]
fn a_signer_can_not_vouch_for_a_line_that_leaves_the_last_accepted_commit() {
    let repo = Repo::new();
    let (laptop, laptop_file) = repo.key("laptop");
    repo.machine("laptop", &laptop, EVERYTHING);
    let first = repo.commit("Signers", None);
    repo.write("types/a/system.toml", "packages = [\"old\"]\n");
    let old = repo.commit("Old, signed", Some(&laptop_file));
    repo.git(&["reset", "-q", "--hard", &first]);
    repo.write("types/a/system.toml", "packages = []\n");
    let anchor = repo.commit("Accepted", Some(&laptop_file));
    repo.git(&["checkout", "-q", "--detach", &old]);
    repo.git(&["merge", "-q", "-s", "ours", "--no-edit", &anchor]);
    repo.commit("Signed on top", Some(&laptop_file));

    let checked = repo.check(&anchor);
    assert_eq!(checked.commit, anchor);
    assert_eq!(
        checked.refused.unwrap().why,
        format!("it doesn't follow {} on main's line", short(&anchor))
    );
}

#[test]
fn a_signer_vouches_only_within_its_reach_and_the_probe_never_vouches() {
    let repo = Repo::new();
    let (laptop, _) = repo.key("laptop");
    let (minipc, minipc_file) = repo.key("minipc");
    let (probe, probe_file) = repo.key("probe");
    let (rogue, rogue_file) = repo.key("rogue");
    repo.machine("laptop", &laptop, EVERYTHING);
    repo.machine("minipc", &minipc, r#"["types/minipc"]"#);
    repo.machine("box", &probe, "versions");
    let service = "types/server/services/db/service.toml";
    repo.write(service, &pinned("16.4.3", OLD));
    repo.write("types/server/services/db/compose.yaml", "services: {}\n");
    let anchor = repo.commit("Signers", None);
    let refused = || {
        let checked = repo.check(&anchor);
        assert_eq!(checked.commit, anchor);
        assert_eq!(checked.refused.unwrap().subject, "Unsigned");
    };

    repo.write("types/server/system.toml", "packages = []\n");
    let unsigned = repo.commit("Unsigned", None);
    repo.write("types/minipc/system.toml", "packages = []\n");
    repo.commit("Vouch for another type", Some(&minipc_file));
    refused();

    repo.git(&["reset", "-q", "--hard", &anchor]);
    repo.write(service, &pinned("16.5.0", NEW));
    repo.commit("Unsigned", None);
    repo.commit("Vouch for versions", Some(&probe_file));
    refused();
    let git = Git::at(repo.work.join(".git"));
    let tip = repo.git(&["rev-parse", "HEAD"]);
    let tree = repo.git(&["rev-parse", "HEAD^{tree}"]);
    let error = signed_commit(&git, &tree, &tip, &anchor, "probe <p@x>", "Again", &probe)
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "the fleet's machines would refuse this commit, so nothing changed: box/probe may \
         change only versions, so it can't vouch for the commits before it"
    );
    let vouched = signed_commit(&git, &tree, &tip, &anchor, "me <m@x>", "Vouch", &laptop);
    assert!(vouched.is_ok());

    repo.git(&["reset", "-q", "--hard", &unsigned]);
    repo.machine("rogue", &rogue, EVERYTHING);
    repo.commit("Let me in", None);
    repo.commit("Vouch for myself", Some(&rogue_file));
    refused();
}

#[test]
fn a_commit_can_not_sign_itself_in() {
    let repo = Repo::new();
    let (laptop, _) = repo.key("laptop");
    let (rogue, rogue_file) = repo.key("rogue");
    repo.machine("laptop", &laptop, EVERYTHING);
    let anchor = repo.commit("Signers", None);
    repo.machine("rogue", &rogue, EVERYTHING);
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
    repo.machine("minipc", &minipc, r#"["types/minipc"]"#);
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

pub const OLD: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
pub const NEW: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";

pub fn pinned(version: &str, digest: &str) -> String {
    format!("[images.db]\nimage = \"postgres\"\nversion = \"{version}\"\ndigest = \"{digest}\"\n")
}

#[test]
fn a_versions_signer_changes_only_versions_and_digests_within_their_major() {
    let repo = Repo::new();
    let (laptop, _) = repo.key("laptop");
    let (probe, probe_file) = repo.key("probe");
    repo.machine("laptop", &laptop, EVERYTHING);
    repo.machine("box", &probe, "versions");
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
            "box/probe can't sign this change to {service}: images.db: 17.0.0 leaves major version 16"
        )
    );
    assert_eq!(
        refused(&pinned("16.4.0", NEW), service),
        format!(
            "box/probe can't sign this change to {service}: images.db: 16.4.0 is older than 16.5.0"
        )
    );
    assert_eq!(
        refused(
            &pinned("16.5.0", OLD).replace("postgres", "evil/postgres"),
            service
        ),
        format!(
            "box/probe can't sign this change to {service}: it changes images.db beyond its version and digest"
        )
    );
    assert_eq!(
        refused(
            "services: {evil: {}}\n",
            "types/server/services/db/compose.yaml"
        ),
        "box/probe can't sign a change to types/server/services/db/compose.yaml"
    );
    assert_eq!(
        refused(
            &pinned("1.0.0", NEW),
            "types/server/services/new/service.toml"
        ),
        "box/probe can't add or remove types/server/services/new/service.toml"
    );
    repo.git(&["reset", "-q", "--hard", &digest]);
    let path = repo.work.join(service);
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(pinned("16.5.0", NEW), &path).unwrap();
    repo.commit("Link", Some(&probe_file));
    assert_eq!(
        repo.check(&digest).refused.unwrap().why,
        format!("box/probe can change only a plain file's text, not {service}'s kind")
    );
    repo.git(&["reset", "-q", "--hard", &digest]);
    fs::remove_file(repo.work.join("machines/box.toml")).unwrap();
    repo.commit("Drop itself", Some(&probe_file));
    assert_eq!(
        repo.check(&digest).refused.unwrap().why,
        "box/probe can't change box, a machine of server"
    );
}

#[test]
fn a_scoped_signer_may_change_a_path_with_non_ascii_characters() {
    let repo = Repo::new();
    let (db, db_file) = repo.key("db");
    repo.machine("db", &db, r#"["types/db"]"#);
    let anchor = repo.commit("Signers", None);
    repo.write("types/db/café", "au lait\n");
    let signed = repo.commit("Add a café", Some(&db_file));

    let checked = repo.check(&anchor);
    assert!(checked.refused.is_none());
    assert_eq!(checked.commit, signed);
    let git = Git::at(repo.work.join(".git"));
    assert_eq!(
        git.changed(&anchor, &signed).unwrap(),
        [PathBuf::from("types/db/café")]
    );
    assert_eq!(
        git.files(&signed, crate::fleet::TYPES).unwrap(),
        [PathBuf::from("types/db/café")]
    );
}
