use crate::{
    fleet::Fleet,
    sync::item::{Change, Guess, Item, Secret},
    util::file,
    util::style,
    util::walk,
};
use anyhow::Result;
use std::{
    fs,
    io::{ErrorKind, Read},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

const READ: u64 = 1 << 20;

enum Shape {
    Marker(&'static str),
    Token { prefix: &'static str, min: usize },
}

const SHAPES: [(Shape, &str); 10] = [
    (Shape::Marker("PRIVATE KEY-----"), "a private key"),
    (Shape::Marker("PRIVATE KEY BLOCK-----"), "a private key"),
    (Shape::Marker("AGE-SECRET-KEY-1"), "an age key"),
    (
        Shape::Token {
            prefix: "ghp_",
            min: 36,
        },
        "a GitHub token",
    ),
    (
        Shape::Token {
            prefix: "gho_",
            min: 36,
        },
        "a GitHub token",
    ),
    (
        Shape::Token {
            prefix: "ghu_",
            min: 36,
        },
        "a GitHub token",
    ),
    (
        Shape::Token {
            prefix: "ghs_",
            min: 36,
        },
        "a GitHub token",
    ),
    (
        Shape::Token {
            prefix: "github_pat_",
            min: 22,
        },
        "a GitHub token",
    ),
    (
        Shape::Token {
            prefix: "sk-",
            min: 20,
        },
        "an API key",
    ),
    (
        Shape::Token {
            prefix: "tskey-",
            min: 20,
        },
        "a Tailscale key",
    ),
];

fn token_in(text: &str) -> Option<&'static str> {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    SHAPES.iter().find_map(|(shape, name)| {
        let found = match shape {
            Shape::Marker(marker) => text.contains(marker),
            Shape::Token { prefix, min } => text.match_indices(prefix).any(|(at, _)| {
                let before = text[..at].chars().next_back();
                let rest = &text[at + prefix.len()..];
                !before.is_some_and(word) && rest.chars().take_while(|c| word(*c)).count() >= *min
            }),
        };
        found.then_some(*name)
    })
}

fn token_at(path: &Path) -> Result<Option<&'static str>> {
    let opened = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)?;
    if !opened.metadata()?.is_file() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    opened.take(READ).read_to_end(&mut bytes)?;
    Ok(token_in(&String::from_utf8_lossy(&bytes)))
}

pub fn finish(fleet: &Fleet, mut item: Item) -> Item {
    let Some(guess) = item.guess.take() else {
        return item;
    };
    let (path, new) = match &guess {
        Guess::New(path) => (path, true),
        Guess::Tracked(path) => (path, false),
    };
    let found = match item.member.map(|i| &fleet.members[i]) {
        None => Found::at(path, new.then_some(Path::new(file::ETC))),
        Some(member) => {
            let user = member.user();
            let base = new.then_some(user.home.as_path());
            user.with_rights(|| Ok(Found::at(path, base)))
                .unwrap_or_else(Found::Unreadable)
        }
    };
    if item.change == Change::Added && item.note.is_empty() {
        item.note = found.describe();
    }
    judge_secret(item, &found, new)
}

enum Found {
    Missing,
    Link(PathBuf),
    Folder {
        files: usize,
        private: bool,
        token: Option<&'static str>,
    },
    File {
        private: bool,
        token: Option<&'static str>,
    },
    Unreadable(anyhow::Error),
}

impl Found {
    fn at(path: &Path, base: Option<&Path>) -> Found {
        let found = match fs::symlink_metadata(path) {
            Err(e) if e.kind() == ErrorKind::NotFound => return Found::Missing,
            found => found,
        };
        let found = found.map_err(anyhow::Error::from).and_then(|meta| {
            if meta.is_symlink() {
                return Ok(Found::Link(fs::read_link(path)?));
            }
            let hidden = match base {
                Some(base) => hidden_below(path, base)?,
                None => false,
            };
            if !meta.is_dir() {
                return Ok(Found::File {
                    private: hidden || walk::private(&meta),
                    token: token_at(path)?,
                });
            }
            let files = walk::files_with_metadata(path)?;
            let mut private = hidden || (base.is_some() && closed(&meta));
            let mut token = None;
            for (rel, meta) in &files {
                private |=
                    walk::private(meta) || (base.is_some() && hidden_below(&path.join(rel), path)?);
                if token.is_none() && meta.is_file() {
                    token = token_at(&path.join(rel))?;
                }
            }
            Ok(Found::Folder {
                files: files.len(),
                private,
                token,
            })
        });
        found.unwrap_or_else(Found::Unreadable)
    }

    fn describe(&self) -> String {
        match self {
            Found::Link(target) => format!("→ {}", target.display()),
            Found::Folder { files, .. } => style::count(*files, "file"),
            Found::Missing | Found::File { .. } => String::new(),
            Found::Unreadable(e) => format!("can't read it: {e:#}"),
        }
    }

    fn private(&self) -> bool {
        match self {
            Found::Missing | Found::Link(_) => false,
            Found::Folder { private, .. } | Found::File { private, .. } => *private,
            Found::Unreadable(_) => true,
        }
    }

    fn token(&self) -> Option<&'static str> {
        match self {
            Found::Folder { token, .. } | Found::File { token, .. } => *token,
            Found::Missing | Found::Link(_) | Found::Unreadable(_) => None,
        }
    }
}

fn hidden_below(path: &Path, base: &Path) -> Result<bool> {
    for dir in path.ancestors().skip(1).take_while(|dir| *dir != base) {
        if !dir.starts_with(base) {
            break;
        }
        if closed(&fs::metadata(dir)?) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn closed(dir: &fs::Metadata) -> bool {
    dir.permissions().mode() & 0o001 == 0
}

fn judge_secret(item: Item, found: &Found, new: bool) -> Item {
    if let Some(token) = found.token() {
        let holds = format!("holds what looks like {token}");
        let note = match item.note.is_empty() {
            true => holds,
            false => format!("{}, {holds}", item.note),
        };
        item.noting(note).secret(Secret::Token)
    } else if new && found.private() {
        item.secret(Secret::Guessed)
    } else {
        item
    }
}

#[cfg(test)]
#[path = "../../../tests/sync/scan/secret.rs"]
mod tests;
