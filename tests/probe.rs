use super::*;
use crate::container::Pin;
use ssh_key::{Algorithm, PrivateKey, rand_core::OsRng};
use std::{fs, process::Command};

fn git(dir: &Path, args: &[&str]) -> String {
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
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

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
    let commit = commit(&repo, &dot_git, &base, &[update], "minipc", &key).unwrap();
    let checked = checked::check(&repo, &base, &commit).unwrap();
    assert!(checked.refused.is_none());
    let text = repo
        .blob(&commit, "types/server/services/db/service.toml")
        .unwrap()
        .unwrap();
    assert!(text.starts_with("# Postgres\n"));
    assert_eq!(container::parse(&text).unwrap()["db"].digest, new);
    assert_eq!(
        git(work, &["log", "-1", "--format=%an <%ae>%n%B", &commit]),
        "minipc/probe <probe@minipc>\nUpdate 1 image\n\nserver/db db: 16.4 to 16.5"
    );
}

fn caddy() -> Watched {
    Watched {
        service: "server/caddy".into(),
        name: "caddy".into(),
        image: container::Image {
            image: "ghcr.io/me/caddy".into(),
            version: "2.8.4".into(),
            digest: String::new(),
            repo: None,
            recipe: true,
        },
        dockerfile: None,
    }
}

fn labelled_by(commit: &str) -> (Client, Pin) {
    use crate::container::registry::tests::{labelled, response, serving};
    let stored = labelled(serde_json::json!({ container::REVISION_LABEL: commit }));
    let pin = Pin {
        version: "2.8.4".into(),
        digest: stored.top.clone(),
    };
    let fake = serving(&stored, Box::new(|_, _, _| response(404, &[], "")));
    (Client::new(Box::new(fake), None).unwrap(), pin)
}

#[test]
fn the_probe_pins_a_recipe_only_when_built_from_the_current_accepted_recipe() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path();
    git(work, &["init", "-q", "-b", "main"]);
    let recipe = work.join("types/server/services/caddy/Dockerfile");
    fs::create_dir_all(recipe.parent().unwrap()).unwrap();
    fs::write(&recipe, "FROM caddy:2.8.4\n").unwrap();
    git(work, &["add", "-A"]);
    git(work, &["commit", "-qm", "Recipe"]);
    let built = git(work, &["rev-parse", "HEAD"]);
    fs::write(work.join("notes"), "").unwrap();
    git(work, &["add", "-A"]);
    git(work, &["commit", "-qm", "Notes"]);
    let tip = git(work, &["rev-parse", "HEAD"]);
    let repo = Git::at(work.join(".git"));

    let (registry, pin) = labelled_by(&built);
    built_from_accepted(&repo, &tip, &registry, &caddy(), &pin).unwrap();

    let (registry, pin) = labelled_by(&"f".repeat(40));
    let err = format!(
        "{:#}",
        built_from_accepted(&repo, &tip, &registry, &caddy(), &pin).unwrap_err()
    );
    assert!(err.contains("isn't on the fleet's main"), "{err}");

    fs::write(&recipe, "FROM caddy:2.8.5\n").unwrap();
    git(work, &["commit", "-qam", "Newer recipe"]);
    let tip = git(work, &["rev-parse", "HEAD"]);
    let (registry, pin) = labelled_by(&built);
    let err = format!(
        "{:#}",
        built_from_accepted(&repo, &tip, &registry, &caddy(), &pin).unwrap_err()
    );
    assert!(err.contains("isn't the current one"), "{err}");

    let (registry, pin) = labelled_by("--upload-pack=x");
    assert!(built_from_accepted(&repo, &tip, &registry, &caddy(), &pin).is_err());
}
