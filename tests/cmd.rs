use super::*;
use std::os::unix::process::CommandExt;

fn sh(script: &str) -> Command {
    let mut command = Command::new("sh");
    command.args(["-c", script]);
    command
}

fn spawn_piped(mut command: Command) -> Child {
    command
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

#[test]
fn returns_output_even_when_a_background_process_holds_the_pipe() {
    let started = Instant::now();
    let out = output(sh("sleep 30 & echo hi"), "sh", &["-c"]).unwrap();
    assert_eq!(out, b"hi\n");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn a_failure_reports_the_stderr() {
    let err = output(sh("echo broken >&2; exit 3"), "sh", &["-c", "x"]).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("sh -c x failed"), "{message}");
    assert!(message.contains("broken"), "{message}");
}

#[test]
fn an_accepted_exit_code_is_not_a_failure() {
    let ok = capture(sh("echo a; exit 1"), "sh", &["-c"], None, Some(1)).unwrap();
    assert_eq!(ok, b"a\n");
    assert!(capture(sh("exit 2"), "sh", &["-c"], None, Some(1)).is_err());
    assert!(capture(sh("exit 1"), "sh", &["-c"], None, None).is_err());
}

#[test]
fn the_limit_kills_the_whole_process_group() {
    let pid_file = tempfile::NamedTempFile::new().unwrap();
    let script = format!("sleep 30 & echo $! > {}; wait", pid_file.path().display());
    let child = spawn_piped(sh(&script));
    let err = finish(child, Some(Duration::from_secs(1)), Sink::Keep).unwrap_err();
    assert!(err.to_string().contains("longer than 1 s"));
    let background: i32 = std::fs::read_to_string(pid_file.path())
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let gone = (0..50).any(|_| {
        thread::sleep(Duration::from_millis(100));
        unsafe { libc::kill(background, 0) == -1 }
    });
    assert!(gone);
}

#[test]
fn a_command_within_its_limit_finishes_normally() {
    let child = spawn_piped(sh("echo ok"));
    let (status, out, _) = finish(child, Some(Duration::from_secs(30)), Sink::Keep).unwrap();
    assert!(status.success());
    assert_eq!(out, b"ok\n");
}
