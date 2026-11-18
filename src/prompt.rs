use anyhow::Result;
use std::io::{self, BufRead, IsTerminal, Write};

pub struct Prompt {
    lines: Box<dyn Iterator<Item = io::Result<String>>>,
    /// Answers piped in show up nowhere else, so a transcript would lose them.
    echo: bool,
}

impl Prompt {
    pub fn new() -> Self {
        Prompt {
            lines: Box::new(io::stdin().lock().lines()),
            echo: !io::stdin().is_terminal(),
        }
    }

    #[cfg(test)]
    pub fn answering(answers: &'static str) -> Self {
        Prompt {
            lines: Box::new(answers.lines().map(|line| Ok(line.to_owned()))),
            echo: false,
        }
    }

    /// None once the input ends.
    pub fn ask(&mut self, question: &str) -> Result<Option<String>> {
        print!("{question} ");
        io::stdout().flush()?;
        let answer = self.lines.next().transpose()?.map(|l| l.trim().to_owned());
        if self.echo {
            println!("{}", answer.as_deref().unwrap_or_default());
        }
        Ok(answer)
    }

    pub fn confirm(&mut self, question: &str) -> Result<bool> {
        Ok(self.ask(&format!("{question} [y/N]?"))?.as_deref() == Some("y"))
    }
}
