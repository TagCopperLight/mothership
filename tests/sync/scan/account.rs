use super::*;

#[test]
fn reads_the_groups_a_sysusers_file_creates() {
    let text = r#"# comment
g docker -
u polkitd 102 "PolicyKit daemon"
u! sshd - "sshd user"
u nm-openvpn 120:nogroup "NetworkManager OpenVPN"
u dnsmasq -:52 "dnsmasq"
m me wheel
r - 500-900
"#;
    assert_eq!(
        sysusers_groups(text),
        ["docker", "polkitd", "sshd", "wheel"]
    );
}
