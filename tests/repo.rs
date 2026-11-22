use super::*;

#[test]
fn only_an_ssh_url_or_a_local_path_is_accepted() {
    for url in [
        "git@github.com:me/fleet.git",
        "ssh://git@github.com/me/fleet",
        "/home/me/fleet.git",
        "file:///home/me/fleet.git",
    ] {
        check_url(url).unwrap();
    }
    for url in [
        "https://github.com/me/fleet",
        "http://example.com/fleet",
        "git://example.com/fleet",
    ] {
        let error = check_url(url).unwrap_err();
        assert!(error.to_string().contains("isn't an SSH URL"), "{url}");
    }
}

#[test]
fn adds_only_the_github_host_keys_not_known_yet() {
    let keys = crate::github::known_hosts(
        br#"{"ssh_keys": ["ssh-ed25519 AAAA", "ssh-rsa BBBB"], "hooks": []}"#,
    )
    .unwrap();
    assert_eq!(
        known_hosts_lines(
            "other.host ssh-ed25519 CCCC\ngithub.com ssh-rsa BBBB\n",
            &keys
        ),
        "github.com ssh-ed25519 AAAA\n"
    );
    assert_eq!(known_hosts_lines("", &keys).lines().count(), 2);
}
