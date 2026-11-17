use crate::{cmd, walk};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

const LINES: usize = 16;
const SNIFF: usize = 8 * 1024;

pub enum Preview {
    None,
    Text(String),
    Contents(PathBuf),
    Diff {
        old: PathBuf,
        new: PathBuf,
    },
    /// Against the version pacman installed.
    Original {
        path: PathBuf,
        archive: PathBuf,
    },
    Chezmoi(PathBuf),
    Unit {
        name: String,
        user: bool,
    },
    Dconf(String),
}

impl Preview {
    pub fn show(&self, color: bool) {
        let lines = match self.render(color) {
            Ok(lines) => lines,
            Err(e) => vec![format!("can't show it: {e:#}")],
        };
        let width = terminal_size::terminal_size().map_or(100, |(w, _)| usize::from(w.0));
        for line in lines.iter().take(LINES) {
            println!("    {}", clip(line, width.saturating_sub(4)));
        }
        if lines.len() > LINES {
            println!("    … {} more lines", lines.len() - LINES);
        }
    }

    fn render(&self, color: bool) -> Result<Vec<String>> {
        Ok(match self {
            Preview::None => Vec::new(),
            Preview::Text(text) => text.lines().map(str::to_owned).collect(),
            Preview::Contents(path) => contents(path)?,
            Preview::Diff { old, new } => diff(old, new, color)?,
            Preview::Original { path, archive } => original(path, archive, color)?,
            Preview::Chezmoi(path) => {
                let color = if color {
                    "--color=true"
                } else {
                    "--color=false"
                };
                let path = path.to_string_lossy();
                // --reverse shows the machine's edits as additions.
                let args = ["diff", "--reverse", "--no-pager", color, &path];
                hunks(&cmd::stdout("chezmoi", &args)?)
            }
            Preview::Unit { name, user } => {
                let scope: &[&str] = if *user { &["--user"] } else { &[] };
                let args = [scope, &["cat", name.as_str()]].concat();
                lines(&cmd::stdout("systemctl", &args)?)
            }
            Preview::Dconf(section) => {
                // dump includes the folders below, which are items of their own.
                let dump = cmd::stdout("dconf", &["dump", &format!("{section}/")])?;
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
    text.lines().map(|l| l.replace('\t', "    ")).collect()
}

fn contents(path: &Path) -> Result<Vec<String>> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_symlink() {
        return Ok(vec![format!("→ {}", fs::read_link(path)?.display())]);
    }
    if meta.is_dir() {
        let files = walk::files(path)?;
        let mut out = vec![format!("folder with {} files", files.len())];
        out.extend(files.iter().map(|f| f.display().to_string()));
        return Ok(out);
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

fn diff(old: &Path, new: &Path, color: bool) -> Result<Vec<String>> {
    let out = Command::new("git")
        .args(["diff", "--no-index", "--no-ext-diff"])
        .arg(if color {
            "--color=always"
        } else {
            "--color=never"
        })
        .args([old, new])
        .output()
        .context("running git diff")?;
    // git diff exits with 1 when the files differ.
    if out.status.code() != Some(0) && out.status.code() != Some(1) {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(hunks(&String::from_utf8_lossy(&out.stdout)))
}

fn original(path: &Path, archive: &Path, color: bool) -> Result<Vec<String>> {
    if !archive.exists() {
        let mut out = vec![format!(
            "pacman's cache no longer has {}, so here is the file:",
            archive.display()
        )];
        out.extend(contents(path)?);
        return Ok(out);
    }
    let member = path.strip_prefix("/")?.to_string_lossy().into_owned();
    let shipped = cmd::bytes("bsdtar", &["-xOf", &archive.to_string_lossy(), &member])?;
    let mut file = tempfile::NamedTempFile::new()?;
    file.write_all(&shipped)?;
    diff(file.path(), path, color)
}

/// The file headers name temporary or source paths.
fn hunks(diff: &str) -> Vec<String> {
    let start = diff.lines().position(|l| strip_ansi(l).starts_with("@@"));
    lines(diff).into_iter().skip(start.unwrap_or(0)).collect()
}

fn strip_ansi(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            chars.by_ref().find(|c| c.is_ascii_alphabetic());
        } else {
            out.push(c);
        }
    }
    out
}

fn clip(line: &str, width: usize) -> String {
    let mut out = String::new();
    let mut visible = 0;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            out.push(c);
            for c in chars.by_ref() {
                out.push(c);
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else if visible < width {
            out.push(c);
            visible += 1;
        }
    }
    if line.contains('\x1b') {
        out.push_str("\x1b[0m");
    }
    out
}
