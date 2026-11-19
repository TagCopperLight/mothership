use super::*;

#[test]
fn only_an_https_url_without_a_login_is_accepted() {
    check_url("https://github.com/me/fleet").unwrap();
    check_url("/home/me/fleet.git").unwrap();
    check_url("file:///home/me/fleet.git").unwrap();
    let login = check_url("https://me:token@github.com/me/fleet").unwrap_err();
    assert!(login.to_string().contains("holds a login"));
}

#[test]
fn ssh_and_plain_http_urls_ask_for_the_https_one() {
    for url in [
        "git@github.com:me/fleet.git",
        "ssh://git@github.com/me/fleet",
        "http://example.com/fleet",
        "git://example.com/fleet",
    ] {
        let error = check_url(url).unwrap_err();
        assert!(error.to_string().contains("give the https URL"), "{url}");
    }
}
