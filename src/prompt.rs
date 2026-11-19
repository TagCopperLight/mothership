use anyhow::{Result, bail};
use std::{
    fmt,
    io::{self, BufRead, IsTerminal, Read, Write},
};

#[derive(Debug)]
pub struct Stopped;

impl fmt::Display for Stopped {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("stopped with Ctrl-C")
    }
}

impl std::error::Error for Stopped {}

pub fn stopped(e: &anyhow::Error) -> bool {
    e.is::<Stopped>()
}

pub struct Choice {
    pub name: String,
    pub about: String,
}

impl Choice {
    pub fn new(name: impl Into<String>, about: impl Into<String>) -> Choice {
        Choice {
            name: name.into(),
            about: about.into(),
        }
    }
}

pub struct Prompt {
    input: Input,
}

enum Input {
    Keys,
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

    pub fn ask(&mut self, question: &str, keys: &[char]) -> Result<Option<char>> {
        print!("{question} ");
        io::stdout().flush()?;
        match &mut self.input {
            Input::Keys => loop {
                match read_key()? {
                    Some(CTRL_C) => {
                        println!();
                        return Err(Stopped.into());
                    }
                    None | Some(CTRL_D) => {
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
            return Ok(matches!(answer.as_deref(), Some("y" | "Y")));
        }
        let keys = ['y', 'Y', 'n', 'N', '\r', '\n'];
        Ok(matches!(self.ask(&question, &keys)?, Some('y' | 'Y')))
    }

    pub fn line(&mut self, question: &str) -> Result<String> {
        self.typed(question, true)
    }

    pub fn hidden(&mut self, question: &str) -> Result<String> {
        self.typed(question, false)
    }

    pub fn choose(&mut self, question: &str, choices: &[Choice]) -> Result<usize> {
        match choices {
            [] => bail!("{question}: there is nothing to choose from"),
            [only] => {
                println!("{question}: {}", only.name);
                return Ok(0);
            }
            _ => {}
        }
        println!("{question}:");
        let width = choices.iter().map(|c| c.name.len()).max().unwrap_or(0);
        for (i, choice) in choices.iter().enumerate() {
            let line = format!("  {}  {:width$}  {}", i + 1, choice.name, choice.about);
            println!("{}", line.trim_end());
        }
        loop {
            let answer = self.line("  ›")?;
            let picked = answer
                .parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .filter(|i| *i < choices.len())
                .or_else(|| choices.iter().position(|c| c.name == answer));
            match picked {
                Some(i) => return Ok(i),
                None => println!(
                    "  Answer with a number from 1 to {}, or a name.",
                    choices.len()
                ),
            }
        }
    }

    fn typed(&mut self, question: &str, shown: bool) -> Result<String> {
        loop {
            print!("{question} ");
            io::stdout().flush()?;
            let answer = match &mut self.input {
                Input::Keys if shown => read_line()?,
                Input::Keys => {
                    let answer = with_terminal(libc::ECHO | libc::ISIG, read_line)?;
                    println!();
                    if answer
                        .as_deref()
                        .is_some_and(|a| a.contains(char::from(CTRL_C)))
                    {
                        return Err(Stopped.into());
                    }
                    answer
                }
                Input::Lines(lines, echo) => {
                    let answer = lines.next().transpose()?;
                    if *echo {
                        println!(
                            "{}",
                            if shown {
                                answer.as_deref().unwrap_or_default()
                            } else {
                                ""
                            }
                        );
                    }
                    answer
                }
            };
            let Some(answer) = answer else {
                bail!("the input ended before an answer to \"{question}\"");
            };
            let answer = match shown {
                true => answer.trim(),
                false => answer.trim_end_matches(['\r', '\n']),
            };
            if !answer.is_empty() {
                return Ok(answer.to_owned());
            }
        }
    }
}

fn read_line() -> Result<Option<String>> {
    let mut line = String::new();
    Ok((io::stdin().lock().read_line(&mut line)? > 0).then_some(line))
}

fn read_key() -> Result<Option<u8>> {
    with_terminal(libc::ICANON | libc::ECHO | libc::ISIG, || {
        let mut byte = [0u8];
        Ok((io::stdin().lock().read(&mut byte)? == 1).then_some(byte[0]))
    })
}

fn with_terminal<T>(off: libc::tcflag_t, read: impl FnOnce() -> Result<T>) -> Result<T> {
    let fd = libc::STDIN_FILENO;
    let mut saved: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut saved) } != 0 {
        return Err(io::Error::last_os_error().into());
    }
    if unsafe { libc::tcflush(fd, libc::TCIFLUSH) } != 0 {
        return Err(io::Error::last_os_error().into());
    }
    let mut raw = saved;
    raw.c_lflag &= !off;
    raw.c_cc[libc::VMIN] = 1;
    raw.c_cc[libc::VTIME] = 0;
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
        return Err(io::Error::last_os_error().into());
    }
    let result = read();
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &saved) } != 0 {
        return Err(io::Error::last_os_error().into());
    }
    result
}

#[cfg(test)]
#[path = "../tests/prompt.rs"]
mod tests;
