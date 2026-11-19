use crate::{
    chezmoi,
    cmd::{self, As},
    package::Archive,
    snapshot::Snapshot,
    style::{self, Paint, Style},
    user::User,
    walk,
};
use anyhow::{Context, Result, bail};
use std::{
    ffi::OsStr,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

const LINES: usize = 16;
const SNIFF: usize = 8 * 1024;
const INDENT: &str = "      ";

pub enum Preview {
    None,
    Contents(PathBuf),
    Home {
        path: PathBuf,
        user: User,
        seen: Seen,
    },
    Diff {
        old: PathBuf,
        new: PathBuf,
    },
    Original {
        path: PathBuf,
        archive: Archive,
    },
    Chezmoi {
        path: PathBuf,
        root: chezmoi::Root,
        seen: Seen,
    },
    Unit {
        name: String,
        user: Option<String>,
    },
    Dconf {
        section: String,
        user: User,
    },
}

#[derive(Default)]
pub struct Seen(OnceLock<String>);

impl Seen {
    fn keep(&self, snapshot: &Snapshot) {
        let _ = self.0.set(snapshot.digest().to_owned());
    }
}

impl Preview {
    pub fn seen(&self) -> Option<&str> {
        match self {
            Preview::Home { seen, .. } | Preview::Chezmoi { seen, .. } => {
                seen.0.get().map(String::as_str)
            }
            _ => None,
        }
    }

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
            Preview::Home { path, user, seen } => {
                let is_dir = user.with_rights(|| Ok(fs::symlink_metadata(path)?.is_dir()))?;
                if is_dir {
                    let files = user.with_rights(|| walk::files(path))?;
                    return Ok(files
                        .iter()
                        .map(|f| style::clean(&f.to_string_lossy()))
                        .collect());
                }
                let snapshot = Snapshot::take(user, &[path])?;
                seen.keep(&snapshot);
                contents(&snapshot.path(user, path)?)?
            }
            Preview::Chezmoi { path, root, seen } => {
                let snapshot = Snapshot::take(root.user(), &[path])?;
                seen.keep(&snapshot);
                let args = ["diff", "--reverse", "--no-pager", "--color=false"].map(OsStr::new);
                let args = [&args[..], &[path.as_os_str()]].concat();
                let diff = String::from_utf8_lossy(&root.output(&snapshot, &args)?).into_owned();
                paint_diff(hunks(&diff), color)
            }
            Preview::Unit { name, user } => {
                let machine = user.as_ref().map(|u| format!("{u}@"));
                let scope: Vec<&str> = match &machine {
                    Some(machine) => vec!["--user", "--machine", machine],
                    None => vec![],
                };
                let args = [&scope[..], &["cat", "--", name.as_str()]].concat();
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
    text.lines().map(style::clean).collect()
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
        let target = fs::read_link(path)?;
        return Ok(vec![format!(
            "→ {}",
            style::clean(&target.to_string_lossy())
        )]);
    }
    if meta.is_dir() {
        let files = walk::files(path)?;
        return Ok(files
            .iter()
            .map(|f| style::clean(&f.to_string_lossy()))
            .collect());
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
