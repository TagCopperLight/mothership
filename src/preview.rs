use crate::{
    cmd::{self, As},
    package::Archive,
    style::{self, Paint, Style},
    user::User,
    walk,
};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

const LINES: usize = 16;
const SNIFF: usize = 8 * 1024;
const INDENT: &str = "      ";

pub enum Preview {
    None,
    Contents(PathBuf),
    Diff { old: PathBuf, new: PathBuf },
    Original { path: PathBuf, archive: Archive },
    Chezmoi { path: PathBuf, user: User },
    Unit { name: String, user: Option<String> },
    Dconf { section: String, user: User },
}

impl Preview {
    pub fn show(&self, color: bool) {
        let lines = match self.render(color) {
            Ok(lines) => lines,
            Err(e) => vec![format!("can't show it: {e:#}")],
        };
        let width = style::width().saturating_sub(INDENT.len());
        for line in lines.iter().take(LINES) {
            println!("{INDENT}{}", style::clip(line, width));
        }
        if lines.len() > LINES {
            println!("{INDENT}… {} more lines", lines.len() - LINES);
        }
    }

    fn render(&self, color: bool) -> Result<Vec<String>> {
        Ok(match self {
            Preview::None => Vec::new(),
            Preview::Contents(path) => contents(path)?,
            Preview::Diff { old, new } => paint_diff(diff(old, new)?, color),
            Preview::Original { path, archive } => paint_diff(original(path, archive)?, color),
            Preview::Chezmoi { path, user } => {
                let path = path.to_string_lossy();
                let args = ["diff", "--reverse", "--no-pager", "--color=false", &path];
                let diff = hunks(&cmd::stdout(As::User(user), "chezmoi", &args)?);
                paint_diff(diff, color)
            }
            Preview::Unit { name, user } => {
                let machine = user.as_ref().map(|u| format!("{u}@"));
                let scope: Vec<&str> = match &machine {
                    Some(machine) => vec!["--user", "--machine", machine],
                    None => vec![],
                };
                let args = [&scope[..], &["cat", name.as_str()]].concat();
                lines(&cmd::stdout(As::Root, "systemctl", &args)?)
            }
            Preview::Dconf { section, user } => {
                let dump = cmd::stdout(As::User(user), "dconf", &["dump", &format!("{section}/")])?;
                lines(&dump)
                    .into_iter()
                    .skip(1)
                    .take_while(|l| !l.starts_with('['))
                    .filter(|l| !l.is_empty())
                    .collect()
            }
        })
    }
}

fn lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| {
            line.replace('\t', "    ")
                .chars()
                .map(|c| if c.is_control() { '\u{fffd}' } else { c })
                .collect()
        })
        .collect()
}

fn paint_diff(lines: Vec<String>, color: bool) -> Vec<String> {
    let style = Style {
        terminal: false,
        color,
    };
    lines
        .into_iter()
        .map(|line| {
            let paint = match line.chars().next() {
                Some('+') => Paint::Green,
                Some('-') => Paint::Red,
                Some('@') => Paint::Cyan,
                _ => return line,
            };
            style.paint(line, &[paint])
        })
        .collect()
}

fn contents(path: &Path) -> Result<Vec<String>> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_symlink() {
        return Ok(vec![format!("→ {}", fs::read_link(path)?.display())]);
    }
    if meta.is_dir() {
        let files = walk::files(path)?;
        return Ok(files.iter().map(|f| f.display().to_string()).collect());
    }
    let mut head = Vec::new();
    fs::File::open(path)?
        .take(SNIFF as u64)
        .read_to_end(&mut head)?;
    if head.contains(&0) {
        return Ok(vec![format!("binary file, {} bytes", meta.len())]);
    }
    Ok(lines(&String::from_utf8_lossy(&fs::read(path)?)))
}

fn diff(old: &Path, new: &Path) -> Result<Vec<String>> {
    let out = Command::new("git")
        .args(["diff", "--no-index", "--no-ext-diff", "--color=never"])
        .args([old, new])
        .output()
        .context("running git diff")?;
    if out.status.code() != Some(0) && out.status.code() != Some(1) {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(hunks(&String::from_utf8_lossy(&out.stdout)))
}

fn original(path: &Path, archive: &Archive) -> Result<Vec<String>> {
    if !archive.path.exists() {
        let mut out = vec![format!(
            "The package cache no longer has {}, so here is the file:",
            archive.path.display()
        )];
        out.extend(contents(path)?);
        return Ok(out);
    }
    let shipped = archive.shipped(path)?;
    let mut file = tempfile::NamedTempFile::new()?;
    file.write_all(&shipped)?;
    diff(file.path(), path)
}

fn hunks(diff: &str) -> Vec<String> {
    let start = diff.lines().position(|l| l.starts_with("@@"));
    lines(diff).into_iter().skip(start.unwrap_or(0)).collect()
}
