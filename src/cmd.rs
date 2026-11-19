use crate::user::User;
use anyhow::{Context, Result, bail};
use std::{
    io::Write,
    process::{Command, Stdio},
};

/// Who runs a program: root, or a user in their own login environment, so their PATH finds what
/// they installed in their home.
#[derive(Clone, Copy)]
pub enum As<'a> {
    Root,
    User(&'a User),
}

impl As<'_> {
    pub fn command(self, program: &str, args: &[&str]) -> Command {
        let mut command = match self {
            As::Root => Command::new(program),
            As::User(user) => {
                let mut runuser = Command::new("runuser");
                runuser
                    .args([
                        "-u",
                        &user.name,
                        "--",
                        "sh",
                        "-lc",
                        "exec \"$0\" \"$@\"",
                        program,
                    ])
                    .current_dir(&user.home);
                runuser
            }
        };
        command.args(args);
        command
    }
}

pub fn stdout(who: As, program: &str, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8(bytes(who, program, args)?)?)
}

pub fn bytes(who: As, program: &str, args: &[&str]) -> Result<Vec<u8>> {
    output(who.command(program, args), program, args)
}

pub fn output(mut command: Command, program: &str, args: &[&str]) -> Result<Vec<u8>> {
    let out = command
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running {program}"))?;
    if !out.status.success() {
        bail!(
            "{program} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

/// Whether the program is on the PATH of whoever runs it.
pub fn found(who: As, program: &str) -> bool {
    bytes(who, "sh", &["-c", "command -v \"$1\"", "sh", program]).is_ok()
}

pub fn run(who: As, program: &str, args: &[&str]) -> Result<()> {
    status(who.command(program, args), program, args)
}

pub fn root(program: &str, args: &[&str]) -> Result<()> {
    run(As::Root, program, args)
}

/// Runs a program that reads a secret on its standard input, so it never shows in `ps`.
pub fn with_input(who: As, program: &str, args: &[&str], input: &[u8]) -> Result<()> {
    let mut child = who
        .command(program, args)
        .stdin(Stdio::piped())
        .spawn()
        .with_context(|| format!("running {program}"))?;
    // A program that stops early closes its input, and its exit status says why. main lets
    // SIGPIPE kill mothership, so the write ignores it.
    // SAFETY: signal only swaps the handler, and the old one goes back right after.
    let previous = unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
    let written = child.stdin.take().expect("piped above").write_all(input);
    unsafe { libc::signal(libc::SIGPIPE, previous) };
    if !child.wait()?.success() {
        bail!("{program} {} failed", args.join(" "));
    }
    written.with_context(|| format!("writing to {program}"))
}

pub fn status(mut command: Command, program: &str, args: &[&str]) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("running {program}"))?;
    if !status.success() {
        bail!("{program} {} failed", args.join(" "));
    }
    Ok(())
}
