use anyhow::{Context, Result, bail};
use std::process::Command;

pub fn stdout(program: &str, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8(bytes(program, args)?)?)
}

pub fn bytes(program: &str, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new(program)
        .args(args)
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
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("running {program}"))?;
    if !status.success() {
        bail!("{program} {} failed", args.join(" "));
    }
    Ok(())
}
