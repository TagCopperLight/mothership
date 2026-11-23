use super::*;
use std::os::unix::fs::symlink;

#[test]
fn finds_the_links_into_a_tool_folder() {
    let dir = tempfile::TempDir::new().unwrap();
    let bin = dir.path().join("bin");
    let tools = dir.path().join("uv/tools");
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(tools.join("optlog/bin")).unwrap();
    symlink(tools.join("optlog/bin/optlog"), bin.join("optlog")).unwrap();
    symlink("/usr/bin/true", bin.join("other")).unwrap();
    fs::write(bin.join("script"), "").unwrap();
    assert_eq!(links_into(&bin, &tools).unwrap(), [bin.join("optlog")]);
}

#[test]
fn tells_a_compiled_program_from_a_script_or_a_link() {
    let dir = tempfile::TempDir::new().unwrap();
    let program = dir.path().join("program");
    let script = dir.path().join("script");
    let link = dir.path().join("link");
    fs::write(&program, b"\x7fELF\x02\x01").unwrap();
    fs::write(&script, "#!/bin/sh\n").unwrap();
    symlink(&program, &link).unwrap();
    assert!(compiled(&program).unwrap());
    assert!(!compiled(&script).unwrap());
    assert!(!compiled(&link).unwrap());
}
