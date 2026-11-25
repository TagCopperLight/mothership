use super::*;
use crate::support::git;
use std::fs;

fn caddy() -> Image {
    Image {
        image: "ghcr.io/me/caddy".into(),
        version: "2.8.4".into(),
        digest: String::new(),
        repo: None,
        recipe: true,
        ports: Vec::new(),
        callers: Default::default(),
    }
}

fn labelled_by(commit: &str) -> (Client, Pin) {
    use crate::services::registry::tests::{labelled, response, serving};
    let stored = labelled(serde_json::json!({ REVISION_LABEL: commit }));
    let pin = Pin {
        version: "2.8.4".into(),
        digest: stored.top.clone(),
    };
    let fake = serving(&stored, Box::new(|_, _, _| response(404, &[], "")));
    (Client::new(Box::new(fake), None).unwrap(), pin)
}

#[test]
fn a_recipe_image_counts_only_when_built_from_the_accepted_recipe() {
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
    let accepted = git(work, &["rev-parse", "HEAD"]);
    let repo = Git::at(work.join(".git"));
    let refusal = |accepted: &str, (registry, pin): (Client, Pin)| {
        built_from(&repo, accepted, &registry, "server/caddy", &caddy(), &pin).unwrap()
    };

    assert_eq!(refusal(&accepted, labelled_by(&built)), None);
    let later = refusal(&accepted, labelled_by(&"f".repeat(40))).unwrap();
    assert!(later.contains("isn't on the fleet's main"), "{later}");

    fs::write(&recipe, "FROM caddy:2.8.5\n").unwrap();
    git(work, &["commit", "-qam", "Newer recipe"]);
    let accepted = git(work, &["rev-parse", "HEAD"]);
    let older = refusal(&accepted, labelled_by(&built)).unwrap();
    assert!(older.contains("isn't the current one"), "{older}");

    let odd = refusal(&accepted, labelled_by("--upload-pack=x")).unwrap();
    assert!(odd.contains("which isn't one"), "{odd}");
}
