use super::*;

#[test]
fn chezmoi_lines_keep_names_that_are_not_utf8() {
    let out = b"/home/me/.bashrc\n/home/me/caf\xe9\n";
    let paths: Vec<&Path> = lines(out).collect();
    assert_eq!(paths[0], Path::new("/home/me/.bashrc"));
    assert_eq!(
        paths[1].as_os_str().as_bytes(),
        b"/home/me/caf\xe9".as_slice()
    );
}
