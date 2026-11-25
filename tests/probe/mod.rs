use super::*;
use crate::{services::Pin, support::git};
use ssh_key::{Algorithm, PrivateKey, rand_core::OsRng};
use std::fs;

#[test]
fn the_probes_commit_passes_the_check_every_machine_runs() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path();
    let key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap();
    git(work, &["init", "-q", "-b", "main"]);
    fs::write(
        work.join("signers.txt"),
        format!("# minipc/probe\n{} versions\n", sign::public(&key).unwrap()),
    )
    .unwrap();
    let service = work.join("types/server/services/db");
    fs::create_dir_all(&service).unwrap();
    let old = format!("sha256:{}", "1".repeat(64));
    fs::write(
        service.join("service.toml"),
        format!("# Postgres\n[images.db]\nimage = \"postgres\"\nversion = \"16.4\"\ndigest = \"{old}\"\n"),
    )
    .unwrap();
    git(work, &["add", "-A"]);
    git(work, &["commit", "-qm", "Fleet"]);
    let base = git(work, &["rev-parse", "HEAD"]);
    let dot_git = work.join(".git");
    let repo = Git::at(&dot_git);
    let new = format!("sha256:{}", "2".repeat(64));
    let update = Update {
        service: "server/db".into(),
        name: "db".into(),
        from: Pin {
            version: "16.4".into(),
            digest: old,
        },
        to: Pin {
            version: "16.5".into(),
            digest: new.clone(),
        },
    };
    let commit = commit(&repo, &dot_git, &base, &base, &[update], "minipc", &key).unwrap();
    let checked = verify::check(&repo, &base, &commit).unwrap();
    assert!(checked.refused.is_none());
    let text = repo
        .blob(&commit, "types/server/services/db/service.toml")
        .unwrap()
        .unwrap();
    assert!(text.starts_with("# Postgres\n"));
    assert_eq!(services::parse(&text).unwrap()["db"].digest, new);
    assert_eq!(
        git(work, &["log", "-1", "--format=%an <%ae>%n%B", &commit]),
        "minipc/probe <probe@minipc>\nUpdate 1 image\n\nserver/db db: 16.4 to 16.5"
    );
}
