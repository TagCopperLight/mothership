use std::{
    fmt::Display,
    io::{self, IsTerminal},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub const CLEAR: &str = "\x1b[2J\x1b[3J\x1b[H";

#[derive(Clone, Copy)]
pub enum Paint {
    Bold,
    Dim,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
}

impl Paint {
    fn code(self) -> u8 {
        match self {
            Paint::Bold => 1,
            Paint::Dim => 2,
            Paint::Red => 31,
            Paint::Green => 32,
            Paint::Yellow => 33,
            Paint::Blue => 34,
            Paint::Magenta => 35,
            Paint::Cyan => 36,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Style {
    pub terminal: bool,
    pub color: bool,
}

impl Style {
    pub fn stdout() -> Self {
        let terminal = io::stdout().is_terminal();
        Style {
            terminal,
            color: terminal && std::env::var_os("NO_COLOR").is_none(),
        }
    }

    pub fn command(self, command: &str) -> String {
        self.paint(format!("sudo mothership {command}"), &[Paint::Cyan])
    }

    pub fn paint(self, text: impl Display, paints: &[Paint]) -> String {
        if !self.color || paints.is_empty() {
            return text.to_string();
        }
        let codes: Vec<String> = paints.iter().map(|p| p.code().to_string()).collect();
        format!("\x1b[{}m{text}\x1b[0m", codes.join(";"))
    }
}

pub fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

pub fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\t' => out.push_str("    "),
            c if c.is_control() || is_invisible(c) => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
    out
}

fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{ad}'
            | '\u{600}'..='\u{605}'
            | '\u{61c}'
            | '\u{6dd}'
            | '\u{70f}'
            | '\u{890}'..='\u{891}'
            | '\u{8e2}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{110bd}'
            | '\u{110cd}'
            | '\u{13430}'..='\u{1343f}'
            | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{e0001}'
            | '\u{e0020}'..='\u{e007f}'
    )
}

pub fn width() -> usize {
    terminal_size::terminal_size().map_or(100, |(w, _)| usize::from(w.0))
}

pub fn visible(line: &str) -> usize {
    strip_ansi(line).width()
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

pub fn wrap(line: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut row = String::new();
    let mut shown = 0;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            row.push(c);
            for c in chars.by_ref() {
                row.push(c);
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        let w = c.width().unwrap_or(0);
        if shown + w > width {
            rows.push(std::mem::take(&mut row));
            shown = 0;
        }
        row.push(c);
        shown += w;
    }
    rows.push(row);
    rows
}

pub fn pad(text: &str, width: usize) -> String {
    let fill = width.saturating_sub(visible(text));
    format!("{text}{}", " ".repeat(fill))
}

#[cfg(test)]
#[path = "../tests/style.rs"]
mod tests;
