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
