use crate::user;
use anyhow::{Context, Result, bail};
use std::process::Command;

pub fn stdout(program: &str, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8(bytes(program, args)?)?)
}

pub fn bytes(program: &str, args: &[&str]) -> Result<Vec<u8>> {
    let out = as_user(program, args)
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

/// Runs a program in this terminal, so it can ask its own questions.
pub fn run(program: &str, args: &[&str]) -> Result<()> {
    status(as_user(program, args), program, args)
}

/// Like `run`, but keeps root under sudo.
pub fn root(program: &str, args: &[&str]) -> Result<()> {
    let mut command = Command::new(program);
    command.args(args);
    status(command, program, args)
}

fn status(mut command: Command, program: &str, args: &[&str]) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("running {program}"))?;
    if !status.success() {
        bail!("{program} {} failed", args.join(" "));
    }
    Ok(())
}

/// chezmoi, cargo, yay and the rest work in the user's home, so under sudo they run as the user.
fn as_user(program: &str, args: &[&str]) -> Command {
    let mut command = if user::is_root() {
        let mut runuser = Command::new("runuser");
        runuser.args(["-u", &user::current().name, "--", program]);
        runuser
    } else {
        Command::new(program)
    };
    command.args(args);
    command
}
