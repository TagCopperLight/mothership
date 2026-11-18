use std::{
    fmt::Display,
    io::{self, IsTerminal},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
    /// A terminal clips long lines and clears review's screen, with or without color.
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

/// "1 file", "3 files".
pub fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

pub fn width() -> usize {
    terminal_size::terminal_size().map_or(100, |(w, _)| usize::from(w.0))
}

/// Columns the text takes on screen: a Chinese character takes two.
pub fn visible(line: &str) -> usize {
    strip_ansi(line).width()
}

pub fn strip_ansi(line: &str) -> String {
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

/// Cuts a line to `width` visible characters, keeping its colors and ending with an ellipsis.
pub fn clip(line: &str, width: usize) -> String {
    if visible(line) <= width {
        return line.to_owned();
    }
    let mut out = String::new();
    let mut shown = 0;
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
        } else {
            let w = c.width().unwrap_or(0);
            if shown + w < width {
                out.push(c);
                shown += w;
            } else {
                shown = width;
            }
        }
    }
    out.push('…');
    if line.contains('\x1b') {
        out.push_str("\x1b[0m");
    }
    out
}

/// Pads to `width` visible characters, so colored text lines up like plain text.
pub fn pad(text: &str, width: usize) -> String {
    let fill = width.saturating_sub(visible(text));
    format!("{text}{}", " ".repeat(fill))
}

#[cfg(test)]
#[path = "../tests/style.rs"]
mod tests;
