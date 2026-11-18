use anyhow::Result;
use std::io::{self, BufRead, IsTerminal, Read, Write};

pub struct Prompt {
    input: Input,
}

enum Input {
    /// A terminal answers with one key, no Enter needed.
    Keys,
    /// Answers piped in show up nowhere else, so a transcript would lose them: they're echoed.
    Lines(Box<dyn Iterator<Item = io::Result<String>>>, bool),
}

const CTRL_C: u8 = 3;
const CTRL_D: u8 = 4;

impl Prompt {
    pub fn new() -> Self {
        let input = if io::stdin().is_terminal() {
            Input::Keys
        } else {
            Input::Lines(Box::new(io::stdin().lock().lines()), true)
        };
        Prompt { input }
    }

    #[cfg(test)]
    pub fn answering(answers: &'static str) -> Self {
        let lines = answers.lines().map(|line| Ok(line.to_owned()));
        Prompt {
            input: Input::Lines(Box::new(lines), false),
        }
    }

    /// Waits for one of `keys`, and returns None when the input ends or I press Ctrl-C.
    pub fn ask(&mut self, question: &str, keys: &[char]) -> Result<Option<char>> {
        print!("{question} ");
        io::stdout().flush()?;
        match &mut self.input {
            Input::Keys => loop {
                match read_key()? {
                    None | Some(CTRL_C | CTRL_D) => {
                        println!();
                        return Ok(None);
                    }
                    Some(byte) if keys.contains(&char::from(byte)) => {
                        let key = char::from(byte);
                        println!("{}", if key.is_ascii_graphic() { key } else { ' ' });
                        return Ok(Some(key));
                    }
                    Some(_) => {}
                }
            },
            Input::Lines(lines, echo) => loop {
                let Some(line) = lines.next().transpose()? else {
                    if *echo {
                        println!();
                    }
                    return Ok(None);
                };
                let line = line.trim();
                if *echo {
                    println!("{line}");
                }
                let mut chars = line.chars();
                if let (Some(c), None) = (chars.next(), chars.next())
                    && keys.contains(&c)
                {
                    return Ok(Some(c));
                }
                print!("{question} ");
                io::stdout().flush()?;
            },
        }
    }

    pub fn confirm(&mut self, question: &str) -> Result<bool> {
        let question = format!("{question} [y/N]?");
        if let Input::Lines(lines, echo) = &mut self.input {
            print!("{question} ");
            io::stdout().flush()?;
            let answer = lines.next().transpose()?.map(|l| l.trim().to_owned());
            if *echo {
                println!("{}", answer.as_deref().unwrap_or_default());
            }
            return Ok(answer.as_deref() == Some("y"));
        }
        Ok(self.ask(&question, &['y', 'n', '\r', '\n'])? == Some('y'))
    }
}

/// Reads one key with the terminal's line editing and echo off, then puts them back.
fn read_key() -> Result<Option<u8>> {
    let fd = libc::STDIN_FILENO;
    // SAFETY: termios is plain data, filled in by tcgetattr before use.
    let mut saved: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut saved) } != 0 {
        return Err(io::Error::last_os_error().into());
    }
    let mut raw = saved;
    raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
    raw.c_cc[libc::VMIN] = 1;
    raw.c_cc[libc::VTIME] = 0;
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
        return Err(io::Error::last_os_error().into());
    }
    let mut byte = [0u8];
    let read = io::stdin().lock().read(&mut byte);
    unsafe { libc::tcsetattr(fd, libc::TCSANOW, &saved) };
    Ok((read? == 1).then_some(byte[0]))
}
