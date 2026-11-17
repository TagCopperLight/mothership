use anyhow::Result;
use std::io::{self, BufRead, Write};

pub struct Prompt {
    lines: Box<dyn Iterator<Item = io::Result<String>>>,
}

impl Prompt {
    pub fn new() -> Self {
        Prompt {
            lines: Box::new(io::stdin().lock().lines()),
        }
    }

    /// Answers each question with the next line of `answers`.
    #[cfg(test)]
    pub fn answering(answers: &'static str) -> Self {
        Prompt {
            lines: Box::new(answers.lines().map(|line| Ok(line.to_owned()))),
        }
    }

    /// None once the input ends.
    pub fn ask(&mut self, question: &str) -> Result<Option<String>> {
        print!("{question} ");
        io::stdout().flush()?;
        Ok(self.lines.next().transpose()?.map(|l| l.trim().to_owned()))
    }

    pub fn confirm(&mut self, question: &str) -> Result<bool> {
        Ok(self.ask(&format!("{question} [y/N]?"))?.as_deref() == Some("y"))
    }

    pub fn choose<'a>(&mut self, question: &str, options: &'a [String]) -> Result<Option<&'a str>> {
        for (n, option) in options.iter().enumerate() {
            println!("    {}. {option}", n + 1);
        }
        let answer = self.ask(&format!("{question} (number, empty to cancel)?"))?;
        Ok(answer
            .and_then(|a| a.parse::<usize>().ok())
            .and_then(|n| options.get(n.checked_sub(1)?))
            .map(String::as_str))
    }
}
