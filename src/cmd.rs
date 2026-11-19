use crate::{style, user::User};
use anyhow::{Context, Result, bail};
use std::{
    ffi::OsStr,
    io::{self, Read, Write},
    ops::{Deref, DerefMut},
    os::unix::process::CommandExt,
    process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const USER_QUERY_LIMIT: Duration = Duration::from_secs(60);
const DRAIN_AFTER_EXIT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
pub enum As<'a> {
    Root,
    User(&'a User),
}

pub struct Cmd {
    command: Command,
    user: bool,
}

impl Deref for Cmd {
    type Target = Command;

    fn deref(&self) -> &Command {
        &self.command
    }
}

impl DerefMut for Cmd {
    fn deref_mut(&mut self) -> &mut Command {
        &mut self.command
    }
}

impl From<Command> for Cmd {
    fn from(command: Command) -> Cmd {
        Cmd {
            command,
            user: false,
        }
    }
}

impl As<'_> {
    pub fn command<S: AsRef<OsStr>>(self, program: &str, args: &[S]) -> Cmd {
        let mut cmd = match self {
            As::Root => Cmd::from(Command::new(program)),
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
                Cmd {
                    command: runuser,
                    user: true,
                }
            }
        };
        cmd.args(args);
        cmd
    }
}

pub fn stdout<S: AsRef<OsStr>>(who: As, program: &str, args: &[S]) -> Result<String> {
    Ok(String::from_utf8(bytes(who, program, args)?)?)
}

pub fn bytes<S: AsRef<OsStr>>(who: As, program: &str, args: &[S]) -> Result<Vec<u8>> {
    output(who.command(program, args), program, args)
}

pub fn output<S: AsRef<OsStr>>(
    command: impl Into<Cmd>,
    program: &str,
    args: &[S],
) -> Result<Vec<u8>> {
    let mut cmd = command.into();
    let limit = cmd.user.then(|| Instant::now() + USER_QUERY_LIMIT);
    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("running {program}"))?;
    let (status, out, err) = finish(child, limit, Sink::Keep)?;
    if !status.success() {
        bail!(
            "{program} {} failed: {}",
            shown(args),
            style::clean(String::from_utf8_lossy(&err).trim())
        );
    }
    Ok(out)
}

pub fn found(who: As, program: &str) -> bool {
    bytes(who, "sh", &["-c", "command -v \"$1\"", "sh", program]).is_ok()
}

pub fn run<S: AsRef<OsStr>>(who: As, program: &str, args: &[S]) -> Result<()> {
    status(who.command(program, args), program, args)
}

pub fn root<S: AsRef<OsStr>>(program: &str, args: &[S]) -> Result<()> {
    run(As::Root, program, args)
}

pub fn with_input<S: AsRef<OsStr>>(who: As, program: &str, args: &[S], input: &[u8]) -> Result<()> {
    let mut cmd = who.command(program, args);
    cmd.stdin(Stdio::piped());
    let mut child = spawn(cmd, program)?;
    let previous = unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
    let written = child.stdin.take().expect("piped above").write_all(input);
    unsafe { libc::signal(libc::SIGPIPE, previous) };
    let (status, ..) = finish(child, None, Sink::Show)?;
    if !status.success() {
        bail!("{program} {} failed", shown(args));
    }
    written.with_context(|| format!("writing to {program}"))
}

pub fn status<S: AsRef<OsStr>>(command: impl Into<Cmd>, program: &str, args: &[S]) -> Result<()> {
    let child = spawn(command.into(), program)?;
    let (status, ..) = finish(child, None, Sink::Show)?;
    if !status.success() {
        bail!("{program} {} failed", shown(args));
    }
    Ok(())
}

fn shown<S: AsRef<OsStr>>(args: &[S]) -> String {
    let args: Vec<_> = args.iter().map(|a| a.as_ref().to_string_lossy()).collect();
    style::clean(&args.join(" "))
}

fn spawn(mut cmd: Cmd, program: &str) -> Result<Child> {
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
    limit: Option<Instant>,
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
        Some(limit) => match exited.recv_timeout(limit.saturating_duration_since(Instant::now())) {
            Ok(status) => status,
            Err(_) => {
                unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
                bail!("it took longer than {} s", USER_QUERY_LIMIT.as_secs());
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
