use super::*;
use crate::trust::{
    sign,
    verify::tests::{EVERYTHING, Repo},
};

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

#[test]
fn a_recovery_install_anchors_at_the_newest_commit_the_recovery_key_signed() {
    let repo = Repo::new();
    let (recovery, recovery_file) = repo.key("recovery");
    let signer = Signer::recovery(&sign::public(&recovery).unwrap()).unwrap();
    let git = Git::at(repo.work.join(".git"));
    let unsigned = repo.commit("Unsigned", None);
    assert!(newest_signed(&git, &unsigned, &signer).is_err());
    let signed = repo.commit("Signed", Some(&recovery_file));
    let (other, other_file) = repo.key("other");
    repo.machine("other", &other, EVERYTHING);
    repo.commit("Signed by another key", Some(&other_file));
    let tip = repo.commit("Pushed with a stolen login", None);
    assert_eq!(newest_signed(&git, &tip, &signer).unwrap(), signed);
}
