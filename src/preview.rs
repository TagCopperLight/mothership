use crate::{
    chezmoi,
    cmd::{self, As},
    package::Archive,
    secrets::Keys,
    snapshot::Snapshot,
    style::{self, Paint, Style},
    user::{self, User},
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
    Unit(String),
    AsUser {
        path: PathBuf,
        user: User,
    },
    Decrypted {
        source: PathBuf,
        keys: Keys,
        now: Option<PathBuf>,
    },
    Locked,
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
        let lines = match self.render() {
            Ok(lines) => lines,
            Err(e) => vec![format!("can't show it: {e:#}")],
        };
        let style = Style {
            terminal: false,
            color: color && self.is_diff(),
        };
        let width = style::width().saturating_sub(INDENT.len());
        for line in lines {
            let paints: &[Paint] = match line.chars().next() {
                Some('+') => &[Paint::Green],
                Some('-') => &[Paint::Red],
                Some('@') => &[Paint::Cyan],
                _ => &[],
            };
            for row in style::wrap(&style::clean(&line), width) {
                println!("{INDENT}{}", style.paint(row, paints));
            }
        }
    }

    fn is_diff(&self) -> bool {
        match self {
            Preview::Diff { .. } | Preview::Original { .. } | Preview::Chezmoi { .. } => true,
            Preview::Decrypted { now, .. } => now.is_some(),
            _ => false,
        }
    }

    fn render(&self) -> Result<Vec<String>> {
        Ok(match self {
            Preview::None => Vec::new(),
            Preview::Locked => vec!["locked: this machine's key can't open it".into()],
            Preview::Contents(path) => contents(path)?,
            Preview::Diff { old, new } => diff(old, new)?,
            Preview::Original { path, archive } => original(path, archive)?,
            Preview::Home { path, user, seen } => {
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
                hunks(&diff)
            }
            Preview::Unit(name) => {
                lines(&cmd::stdout(As::Root, "systemctl", &["cat", "--", name])?)
            }
            Preview::AsUser { path, user } => user.with_rights(|| contents(path))?,
            Preview::Decrypted { source, keys, now } => {
                let plain = keys.decrypt(source)?;
                match now {
                    Some(now) => diff_bytes(&plain, now)?,
                    None => {
                        let mut file = tempfile::NamedTempFile::new()?;
                        file.write_all(&plain)?;
                        contents(file.path())?
                    }
                }
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
    text.lines().map(str::to_owned).collect()
}

fn contents(path: &Path) -> Result<Vec<String>> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_symlink() {
        let target = fs::read_link(path)?;
        return Ok(vec![format!("→ {}", target.to_string_lossy())]);
    }
    if meta.is_dir() {
        let mut out = Vec::new();
        for rel in walk::files(path)? {
            out.push(format!("── {}", rel.to_string_lossy()));
            out.extend(contents(&path.join(rel))?);
        }
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

fn diff(old: &Path, new: &Path) -> Result<Vec<String>> {
    let out = Command::new("git")
        .args(["diff", "--no-index", "--no-ext-diff", "--color=never"])
        .arg("--no-textconv")
        .args([old, new])
        .current_dir("/")
        .env_clear()
        .envs([
            ("PATH", user::SYSTEM_PATH),
            ("LANG", "C.UTF-8"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ])
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
    diff_bytes(&archive.shipped(path)?, path)
}

fn diff_bytes(old: &[u8], new: &Path) -> Result<Vec<String>> {
    let mut file = tempfile::NamedTempFile::new()?;
    file.write_all(old)?;
    diff(file.path(), new)
}

fn hunks(diff: &str) -> Vec<String> {
    let start = diff.lines().position(|l| l.starts_with("@@"));
    lines(diff).into_iter().skip(start.unwrap_or(0)).collect()
}

#[cfg(test)]
#[path = "../tests/preview.rs"]
mod tests;
