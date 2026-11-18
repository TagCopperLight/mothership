use super::*;

fn git(name: &str, url: &str, branch: Option<&str>) -> Source {
    Source::Declarable(Tool::Git(GitTool {
        name: name.into(),
        git: url.into(),
        branch: branch.map(str::to_owned),
    }))
}

fn registry(name: &str) -> Source {
    Source::Declarable(Tool::Name(name.into()))
}

#[test]
fn reads_cargo_sources() {
    let list = "\\
cargo-nextest v0.9.146:
    cargo-nextest
chromash v0.1.0 (https://github.com/me/Chromash?branch=add-splash-false#59fe87f3):
    chromash
hello v0.1.0 (file:///crates/hello#1a2b3c4d):
    hello
pinned v0.1.0 (https://github.com/me/pinned?tag=v1#1a2b3c4d):
    pinned
local v0.1.0 (/home/me/local):
    local
";
    let tools = cargo_list(list);
    assert_eq!(tools["cargo-nextest"], registry("cargo-nextest"));
    assert_eq!(
        tools["chromash"],
        git(
            "chromash",
            "https://github.com/me/Chromash",
            Some("add-splash-false")
        )
    );
    assert_eq!(tools["hello"], git("hello", "file:///crates/hello", None));
    assert!(matches!(tools["pinned"], Source::Other(_)));
    assert!(matches!(tools["local"], Source::Other(_)));
}

#[test]
fn reads_uv_receipts() {
    let receipt = |requirement: &str| {
        uv_receipt(&format!("[tool]\nrequirements = [{requirement}]\n"))
            .unwrap()
            .1
    };
    assert_eq!(receipt(r#"{ name = "ruff" }"#), registry("ruff"));
    assert_eq!(
        receipt(r#"{ name = "pyx", git = "https://example.com/pyx" }"#),
        git("pyx", "https://example.com/pyx", None)
    );
    assert_eq!(
        receipt(r#"{ name = "pyx", git = "https://example.com/pyx?rev=dev" }"#),
        git("pyx", "https://example.com/pyx", Some("dev"))
    );
    for other in [
        r#"{ name = "optlog", editable = "/home/me/optlog" }"#,
        r#"{ name = "ruff", specifier = "==0.5" }"#,
        r#"{ name = "pyx", git = "https://example.com/pyx?tag=v1" }"#,
    ] {
        assert!(matches!(receipt(other), Source::Other(_)), "{other}");
    }
}

#[test]
fn reads_bun_globals() {
    assert_eq!(bun_source("cowsay", "^1.6.0"), registry("cowsay"));
    assert_eq!(
        bun_source("js", "git+https://example.com/js.git#dev"),
        git("js", "https://example.com/js.git", Some("dev"))
    );
    assert_eq!(
        bun_source("js", "git+https://example.com/js.git"),
        git("js", "https://example.com/js.git", None)
    );
    assert!(matches!(bun_source("x", "file:../x"), Source::Other(_)));
    assert!(matches!(bun_source("x", "github:me/x"), Source::Other(_)));
}
