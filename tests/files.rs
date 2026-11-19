use super::*;
use std::path::Path;

#[test]
fn status_lines_need_two_codes_a_space_and_a_path() {
    assert_eq!(status_line("MM .bashrc"), Some((b'M', b'M', ".bashrc")));
    assert_eq!(status_line(" A dir/file"), Some((b' ', b'A', "dir/file")));
    assert_eq!(status_line("junk"), None);
    assert_eq!(status_line("MM"), None);
    assert_eq!(status_line("MMx.bashrc"), None);
}

#[test]
fn chezmoi_paths_stay_in_the_home() {
    let user = crate::user::by_uid(unsafe { libc::geteuid() }).unwrap();
    let home = &user.home;
    assert!(chezmoi::in_home(&user, &home.join(".config/app")).is_ok());
    assert!(chezmoi::in_home(&user, &home.join("../other/.ssh")).is_err());
    assert!(chezmoi::in_home(&user, Path::new("/etc/passwd")).is_err());
}
