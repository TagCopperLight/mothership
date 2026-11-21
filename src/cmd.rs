use crate::{style, user::User};
use anyhow::{Context, Result, bail};
use std::{
    ffi::OsStr,
    io::{self, Read, Write},
    os::unix::process::CommandExt,
    process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const USER_QUERY_LIMIT: Duration = Duration::from_secs(60);
const USER_RUN_LIMIT: Duration = Duration::from_secs(2 * 60 * 60);
const DRAIN_AFTER_EXIT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
pub enum As<'a> {
    Root,
    User(&'a User),
}

impl As<'_> {
    pub fn command<S: AsRef<OsStr>>(self, program: &str, args: &[S]) -> Command {
        let mut command = match self {
            As::Root => Command::new(program),
            As::User(user) => {
                let mut runuser = Command::new("runuser");
                runuser
                    .args(["-u", &user.name, "--", program])
                    .current_dir(&user.home)
                    .env_clear()
                    .envs(user.environment())
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                unsafe {
                    runuser.pre_exec(|| match libc::setsid() {
                        -1 => Err(io::Error::last_os_error()),
                        _ => Ok(()),
                    });
                }
                runuser
            }
        };
        command.args(args);
        command
    }

    fn limit(self, user_limit: Duration) -> Option<Duration> {
        match self {
            As::Root => None,
            As::User(_) => Some(user_limit),
        }
    }
}

pub fn stdout<S: AsRef<OsStr>>(who: As, program: &str, args: &[S]) -> Result<String> {
    Ok(String::from_utf8(bytes(who, program, args)?)?)
}

pub fn bytes<S: AsRef<OsStr>>(who: As, program: &str, args: &[S]) -> Result<Vec<u8>> {
    let limit = who.limit(USER_QUERY_LIMIT);
    capture(who.command(program, args), program, args, limit, None, None)
}

pub fn bytes_allowing_exit<S: AsRef<OsStr>>(
    who: As,
    program: &str,
    args: &[S],
    code: i32,
) -> Result<Vec<u8>> {
    let limit = who.limit(USER_QUERY_LIMIT);
    capture(
        who.command(program, args),
        program,
        args,
        limit,
        Some(code),
        None,
    )
}

pub fn output<S: AsRef<OsStr>>(command: Command, program: &str, args: &[S]) -> Result<Vec<u8>> {
    capture(command, program, args, None, None, None)
}

pub fn output_within<S: AsRef<OsStr>>(
    command: Command,
    program: &str,
    args: &[S],
    limit: Duration,
) -> Result<Vec<u8>> {
    capture(command, program, args, Some(limit), None, None)
}

pub fn status_within<S: AsRef<OsStr>>(
    command: Command,
    program: &str,
    args: &[S],
    limit: Duration,
) -> Result<()> {
    show(command, program, args, Some(limit))
}

pub fn output_with_input<S: AsRef<OsStr>>(
    command: Command,
    program: &str,
    args: &[S],
    input: &[u8],
) -> Result<Vec<u8>> {
    capture(command, program, args, None, None, Some(input))
}

fn capture<S: AsRef<OsStr>>(
    mut command: Command,
    program: &str,
    args: &[S],
    limit: Option<Duration>,
    also_ok: Option<i32>,
    input: Option<&[u8]>,
) -> Result<Vec<u8>> {
    let mut child = command
        .stdin(match input {
            Some(_) => Stdio::piped(),
            None => Stdio::null(),
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("running {program}"))?;
    let writer = input.map(|bytes| feed(&mut child, bytes));
    let finished = finish(child, limit, Sink::Keep);
    let written = writer.map(|w| w.join().expect("the writing thread doesn't panic"));
    let (status, out, err) = finished?;
    if !status.success() && status.code() != also_ok {
        bail!(
            "{program} {} failed: {}",
            shown(args),
            style::clean(String::from_utf8_lossy(&err).trim())
        );
    }
    if let Some(written) = written {
        written.with_context(|| format!("writing to {program}"))?;
    }
    Ok(out)
}

pub fn found(who: As, program: &str) -> bool {
    bytes(who, "sh", &["-c", "command -v \"$1\"", "sh", program]).is_ok()
}

pub fn run<S: AsRef<OsStr>>(who: As, program: &str, args: &[S]) -> Result<()> {
    show(
        who.command(program, args),
        program,
        args,
        who.limit(USER_RUN_LIMIT),
    )
}

pub fn root<S: AsRef<OsStr>>(program: &str, args: &[S]) -> Result<()> {
    run(As::Root, program, args)
}

pub fn with_input<S: AsRef<OsStr>>(who: As, program: &str, args: &[S], input: &[u8]) -> Result<()> {
    let mut cmd = who.command(program, args);
    cmd.stdin(Stdio::piped());
    let mut child = spawn(cmd, program)?;
    let writer = feed(&mut child, input);
    let (status, ..) = finish(child, who.limit(USER_RUN_LIMIT), Sink::Show)?;
    let written = writer.join().expect("the writing thread doesn't panic");
    if !status.success() {
        bail!("{program} {} failed", shown(args));
    }
    written.with_context(|| format!("writing to {program}"))
}

fn feed(child: &mut Child, input: &[u8]) -> thread::JoinHandle<io::Result<()>> {
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let input = input.to_vec();
    thread::spawn(move || {
        unsafe {
            let mut pipe: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut pipe);
            libc::sigaddset(&mut pipe, libc::SIGPIPE);
            libc::pthread_sigmask(libc::SIG_BLOCK, &pipe, std::ptr::null_mut());
        }
        stdin.write_all(&input)
    })
}

pub fn status<S: AsRef<OsStr>>(command: Command, program: &str, args: &[S]) -> Result<()> {
    show(command, program, args, None)
}

fn show<S: AsRef<OsStr>>(
    command: Command,
    program: &str,
    args: &[S],
    limit: Option<Duration>,
) -> Result<()> {
    let child = spawn(command, program)?;
    let (status, ..) = finish(child, limit, Sink::Show)?;
    if !status.success() {
        bail!("{program} {} failed", shown(args));
    }
    Ok(())
}

fn shown<S: AsRef<OsStr>>(args: &[S]) -> String {
    let args: Vec<_> = args.iter().map(|a| a.as_ref().to_string_lossy()).collect();
    style::clean(&args.join(" "))
}

fn spawn(mut cmd: Command, program: &str) -> Result<Child> {
    cmd.spawn().with_context(|| format!("running {program}"))
}

#[derive(Clone, Copy)]
enum Sink {
    Show,
    Keep,
}

enum Chunk {
    Out(Vec<u8>),
    Err(Vec<u8>),
    Closed,
}

fn finish(
    mut child: Child,
    limit: Option<Duration>,
    sink: Sink,
) -> Result<(ExitStatus, Vec<u8>, Vec<u8>)> {
    let (tx, chunks) = mpsc::channel();
    let mut open = 0;
    if let Some(pipe) = child.stdout.take() {
        read_pipe(Pipe::Out(pipe), sink, tx.clone());
        open += 1;
    }
    if let Some(pipe) = child.stderr.take() {
        read_pipe(Pipe::Err(pipe), sink, tx.clone());
        open += 1;
    }
    drop(tx);
    let pid = child.id();
    let (done, exited) = mpsc::channel();
    thread::spawn(move || done.send(child.wait()));
    let status = match limit {
        None => exited.recv()?,
        Some(limit) => match exited.recv_timeout(limit) {
            Ok(status) => status,
            Err(_) => {
                unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
                bail!("it took longer than {} s", limit.as_secs());
            }
        },
    }?;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let drained = Instant::now() + DRAIN_AFTER_EXIT;
    while open > 0 {
        match chunks.recv_timeout(drained.saturating_duration_since(Instant::now())) {
            Ok(Chunk::Out(bytes)) => out.extend(bytes),
            Ok(Chunk::Err(bytes)) => err.extend(bytes),
            Ok(Chunk::Closed) => open -= 1,
            Err(_) => break,
        }
    }
    Ok((status, out, err))
}

enum Pipe {
    Out(ChildStdout),
    Err(ChildStderr),
}

fn read_pipe(pipe: Pipe, sink: Sink, tx: mpsc::Sender<Chunk>) {
    thread::spawn(move || {
        let (mut reader, to_err): (Box<dyn Read + Send>, bool) = match pipe {
            Pipe::Out(pipe) => (Box::new(pipe), false),
            Pipe::Err(pipe) => (Box::new(pipe), true),
        };
        let mut shown = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    let message = format!("\nreading the command's output failed: {e}\n");
                    let _ = tx.send(Chunk::Err(message.into_bytes()));
                    break;
                }
            };
            let bytes = &buf[..n];
            match sink {
                Sink::Keep if to_err => drop(tx.send(Chunk::Err(bytes.to_vec()))),
                Sink::Keep => drop(tx.send(Chunk::Out(bytes.to_vec()))),
                Sink::Show => {
                    shown.extend_from_slice(bytes);
                    show_lines(&mut shown, to_err, false);
                }
            }
        }
        show_lines(&mut shown, to_err, true);
        let _ = tx.send(Chunk::Closed);
    });
}

fn show_lines(pending: &mut Vec<u8>, to_err: bool, all: bool) {
    let end = match all {
        true => pending.len(),
        false => match pending.iter().rposition(|b| matches!(b, b'\n' | b'\r')) {
            Some(i) => i + 1,
            None => return,
        },
    };
    let text: String = String::from_utf8_lossy(&pending[..end])
        .split_inclusive(['\n', '\r'])
        .map(|line| {
            let (body, end) = match line.strip_suffix(['\n', '\r']) {
                Some(body) => (body, &line[body.len()..]),
                None => (line, ""),
            };
            format!("{}{end}", style::clean(body))
        })
        .collect();
    pending.drain(..end);
    let written = match to_err {
        true => io::stderr().write_all(text.as_bytes()),
        false => io::stdout().write_all(text.as_bytes()),
    };
    drop(written);
}

#[cfg(test)]
#[path = "../tests/cmd.rs"]
mod tests;
