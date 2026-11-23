use super::*;
use crate::fleet::Tools;
use strum::IntoEnumIterator;

fn first(manager: Manager, text: &str) -> Tool {
    let tools: Tools = toml::from_str(text).unwrap();
    tools.of(manager)[0].clone()
}

#[test]
fn installs_from_the_registry_by_name() {
    for manager in Manager::iter() {
        let text = format!(r#"{} = ["x"]"#, manager.name());
        assert_eq!(spec(manager, &first(manager, &text)), ["x"]);
    }
}

#[test]
fn installs_from_git_with_or_without_a_branch() {
    let git = |manager: Manager, branch: &str| {
        let text = format!(
            r#"{} = [{{ name = "x", git = "https://example.com/x"{branch} }}]"#,
            manager.name()
        );
        spec(manager, &first(manager, &text))
    };
    let branch = r#", branch = "dev""#;
    assert_eq!(
        git(Manager::Cargo, ""),
        ["--git", "https://example.com/x", "x"]
    );
    assert_eq!(
        git(Manager::Cargo, branch),
        ["--git", "https://example.com/x", "--branch", "dev", "x"]
    );
    assert_eq!(git(Manager::Uv, ""), ["x @ git+https://example.com/x"]);
    assert_eq!(
        git(Manager::Uv, branch),
        ["x @ git+https://example.com/x@dev"]
    );
    assert_eq!(git(Manager::Bun, ""), ["git+https://example.com/x"]);
    assert_eq!(git(Manager::Bun, branch), ["git+https://example.com/x#dev"]);
}

#[test]
fn a_tool_with_an_unknown_field_is_an_error() {
    let text = r#"uv = [{ name = "x", git = "https://example.com/x", editable = "~/x" }]"#;
    assert!(toml::from_str::<Tools>(text).is_err());
}
