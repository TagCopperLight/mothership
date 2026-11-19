use super::*;

fn parsed(url: &str) -> Option<(Scheme, Option<&str>, &str, u16)> {
    remote(url)
        .unwrap()
        .map(|r| (r.scheme, r.userinfo, r.host, r.port))
}

#[test]
fn finds_where_a_url_connects() {
    assert_eq!(
        parsed("https://github.com/me/fleet"),
        Some((Scheme::Https, None, "github.com", 443))
    );
    assert_eq!(
        parsed("https://me:token@example.com:8443/fleet.git"),
        Some((Scheme::Https, Some("me:token"), "example.com", 8443))
    );
    assert_eq!(
        parsed("git@github.com:me/fleet.git"),
        Some((Scheme::Ssh, Some("git"), "github.com", 22))
    );
    assert_eq!(
        parsed("ssh://git@[::1]:2222/fleet"),
        Some((Scheme::Ssh, Some("git"), "[::1]", 2222))
    );
    assert_eq!(
        parsed("http://[::1]/fleet"),
        Some((Scheme::Http, None, "[::1]", 80))
    );
    assert_eq!(parsed("/home/me/fleet.git"), None);
    assert_eq!(parsed("file:///home/me/fleet.git"), None);
    assert!(remote("ftp://example.com/fleet").is_err());
}

#[test]
fn reads_the_first_apt_source() {
    assert_eq!(
        apt_uri("URIs: http://deb.debian.org/debian http://other"),
        Some("http://deb.debian.org/debian")
    );
    assert_eq!(
        apt_uri("deb [trusted=yes arch=amd64] copy:/repo ./"),
        Some("copy:/repo")
    );
    assert_eq!(
        apt_uri("deb http://archive.ubuntu.com/ubuntu noble main"),
        Some("http://archive.ubuntu.com/ubuntu")
    );
    assert_eq!(apt_uri("# deb http://old"), None);
}

#[test]
fn waiting_for_the_network_gives_up_after_its_patience() {
    let error = wait_for("127.0.0.1:1", Some(Duration::ZERO)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "127.0.0.1:1 still doesn't answer: install stops, and goes on at the next start"
    );
}
