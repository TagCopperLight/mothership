use super::*;
use std::path::Path;

#[test]
fn status_lines_need_two_codes_a_space_and_a_path() {
    fn line(code: &[u8], path: &'static str) -> Option<(u8, u8, &'static Path)> {
        Some((code[0], code[1], Path::new(path)))
    }
    assert_eq!(status_line(b"MM .bashrc"), line(b"MM", ".bashrc"));
    assert_eq!(status_line(b" A dir/file"), line(b" A", "dir/file"));
    assert_eq!(status_line(b"junk"), None);
    assert_eq!(status_line(b"MM"), None);
    assert_eq!(status_line(b"MM "), None);
    assert_eq!(status_line(b"MMx.bashrc"), None);
    let (_, _, odd) = status_line(b"MM caf\xe9").unwrap();
    assert_eq!(odd.as_os_str().as_bytes(), b"caf\xe9");
}

#[test]
fn chezmoi_paths_stay_in_the_home() {
    let user = crate::support::current_user();
    let home = &user.home;
    assert!(chezmoi::in_home(&user, &home.join(".config/app")).is_ok());
    assert!(chezmoi::in_home(&user, &home.join("../other/.ssh")).is_err());
    assert!(chezmoi::in_home(&user, Path::new("/etc/passwd")).is_err());
}
