use super::*;

#[test]
fn reads_the_groups_a_sysusers_file_creates() {
    let text = r#"# comment
g docker -
u polkitd 102 "PolicyKit daemon"
m me wheel
r - 500-900
"#;
    assert_eq!(sysusers_groups(text), ["docker", "polkitd", "wheel"]);
}
