use super::*;

fn listed(id: u64, title: &str, key: &str) -> Listed {
    Listed {
        id,
        key: key.into(),
        title: title.into(),
    }
}

fn key(title: &str, public: &str, kind: Use) -> Key {
    Key {
        title: title.into(),
        public: public.into(),
        kind,
    }
}

#[test]
fn only_a_fleet_on_github_over_ssh_reaches_github() {
    assert_eq!(
        repo("git@github.com:me/fleet.git").as_deref(),
        Some("me/fleet")
    );
    assert_eq!(
        repo("ssh://git@github.com/me/fleet").as_deref(),
        Some("me/fleet")
    );
    assert_eq!(repo("/srv/fleet.git"), None);
    assert_eq!(repo("git@github.com.evil:me/fleet"), None);
}

#[test]
fn adds_only_the_keys_github_lacks() {
    let keys = [
        key("desktop/me", "ssh-ed25519 A", Use::Login),
        key("desktop", "ssh-ed25519 B", Use::Signing),
        key("desktop/dev", "ssh-ed25519 C", Use::Login),
    ];
    let titles = |kind: Use, known: &[Listed]| -> Vec<String> {
        let wanted: Vec<&Key> = keys.iter().filter(|key| key.kind == kind).collect();
        missing(&wanted, known)
            .iter()
            .map(|key| key.title.clone())
            .collect()
    };
    let login = [listed(1, "old title", "ssh-ed25519 A")];
    assert_eq!(titles(Use::Login, &login), ["desktop/dev"]);
    let signing = [listed(2, "desktop", "ssh-ed25519 A")];
    assert_eq!(titles(Use::Signing, &signing), ["desktop"]);
}

#[test]
fn retiring_a_host_drops_its_keys_by_title() {
    let login = [
        listed(1, "desktop/me", "a"),
        listed(2, "desktop-old/me", "b"),
        listed(3, "desktop", "c"),
    ];
    assert_eq!(
        retired("desktop", Use::Login, &login),
        [(1, "desktop/me".to_owned())]
    );
    let signing = [listed(4, "desktop", "d"), listed(5, "desktop/me", "e")];
    assert_eq!(
        retired("desktop", Use::Signing, &signing),
        [(4, "desktop".to_owned())]
    );
}
