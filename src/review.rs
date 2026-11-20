use crate::{
    edit::{self, Add},
    fleet::{
        Fleet,
        checked::{self, Advance},
    },
    host,
    item::{Adding, Item, Secret, Section},
    later::{self, DAY, Later},
    prompt::{self, Prompt},
    record::Record,
    revert::{self, Etc, Machine, Revert},
    scan::{self, Columns},
    sign,
    style::{self, Paint, Style},
    walk,
};
use anyhow::{Error, Result};

pub fn run() -> Result<()> {
    let stage = checked::stage()?;
    let report = scan::collect(&stage.dir)?;
    let mut prompt = Prompt::new();
    let result = ask(&report, &mut prompt);
    let message = match &result {
        Ok(tally) if !tally.parts().is_empty() => {
            format!("Review on {}: {}", host::name()?, tally.parts().join(", "))
        }
        _ => format!("Review on {}", host::name()?),
    };
    let admin = report.fleet.admin();
    let committed = sign::require_machine_key()
        .and_then(|key| stage.commit(admin, &message, &key, &mut prompt, Advance::Merge));
    result?;
    committed.map(drop)
}

#[derive(Default)]
struct Tally {
    added: usize,
    reverted: usize,
    ignored: usize,
    put_off: usize,
    passed: usize,
    not_secret: usize,
}

impl Tally {
    fn counts(&self) -> [(usize, &'static str, Paint); 5] {
        [
            (self.added, "added", Paint::Green),
            (self.reverted, "reverted", Paint::Yellow),
            (self.ignored, "ignored", Paint::Dim),
            (self.put_off, "put off for later", Paint::Blue),
            (self.passed, "passed", Paint::Dim),
        ]
    }

    fn parts(&self) -> Vec<String> {
        [
            (self.added, "added"),
            (self.ignored, "ignored"),
            (self.not_secret, "marked not secret"),
        ]
        .iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, what)| format!("{count} {what}"))
        .collect()
    }

    fn count(&mut self, answer: Answer) {
        let count = match answer {
            Answer::Add(_) | Answer::Secret(_) => &mut self.added,
            Answer::NotSecret => &mut self.not_secret,
            Answer::Revert(_) => &mut self.reverted,
            Answer::Ignore => &mut self.ignored,
            Answer::Later => &mut self.put_off,
            Answer::Pass => &mut self.passed,
            Answer::Quit => return,
        };
        *count += 1;
    }
}

#[derive(Clone, Copy)]
enum Answer<'a> {
    Add(&'a Add),
    Secret(&'a Add),
    NotSecret,
    Revert(&'a Revert),
    Ignore,
    Later,
    Pass,
    Quit,
}

impl Answer<'_> {
    const ALWAYS: [Answer<'static>; 4] =
        [Answer::Ignore, Answer::Later, Answer::Pass, Answer::Quit];

    fn key(self) -> char {
        match self {
            Answer::Add(_) => 'a',
            Answer::Secret(_) => 's',
            Answer::NotSecret => 'n',
            Answer::Revert(_) => 'r',
            Answer::Ignore => 'i',
            Answer::Later => 'l',
            Answer::Pass => 'p',
            Answer::Quit => 'q',
        }
    }

    fn name(self) -> &'static str {
        match self {
            Answer::Add(_) => "add",
            Answer::Secret(_) => "secret",
            Answer::NotSecret => "not secret",
            Answer::Revert(_) => "revert",
            Answer::Ignore => "ignore",
            Answer::Later => "later",
            Answer::Pass => "pass",
            Answer::Quit => "quit",
        }
    }
}

fn ask(report: &scan::Report, prompt: &mut Prompt) -> Result<Tally> {
    let fleet = &report.fleet;
    let style = Style::stdout();
    let mut later = Later::load()?;
    let mut record = Record::load()?;
    let now = later::now()?;
    let back_later = format!(
        "back in {}",
        style::count((later::LATER / DAY) as usize, "day")
    );
    let mine: Vec<&Item> = report
        .shown
        .iter()
        .filter(|item| item.section != Section::Fleet)
        .collect();
    let due: Vec<&Item> = mine
        .iter()
        .copied()
        .filter(|item| later.due(fleet, item, now))
        .collect();

    let mut tally = Tally::default();
    let mut last: Option<String> = None;
    let mut seen = 0;
    let mut failure: Option<Error> = None;
    'items: for (n, item) in due.iter().enumerate() {
        if style.terminal {
            print!("{}", style::CLEAR);
            if let Some(last) = last.take() {
                println!("{}\n", style.paint(last, &[Paint::Dim]));
            }
        }
        seen = n;
        let member = item.member.map(|i| &fleet.members[i]);
        let mut header = format!("[{}/{}] {}", n + 1, due.len(), scan::heading(fleet, item));
        if style.terminal {
            header = format!(
                "{}  {}",
                style.paint(header, &[Paint::Bold]),
                progress(n, due.len(), style)
            );
        }
        println!("{header}");
        println!("{}", scan::line(item, style, Columns::of(&[item]), None));
        if let Some(waiting) = later.get(fleet, item) {
            let days = style::count((now.saturating_sub(waiting.first) / DAY) as usize, "day");
            let ago = format!("put off {days} ago");
            println!("      {}", style.paint(ago, &[Paint::Blue]));
        }
        item.preview.show(&item.seen, style.color);

        let add = match &item.add {
            Adding::Offered(add) => Some(add),
            Adding::Refused(_) | Adding::Never => None,
        };
        let encrypted = add
            .filter(|_| item.secret == Secret::No)
            .and_then(Add::encrypted);
        let choices = choices(item, add, encrypted.as_ref());
        let key = |answer: Answer| style.paint(answer.key(), &[Paint::Bold, Paint::Cyan]);
        for (answer, what) in &choices {
            println!("  {}  {:10}  {what}", key(*answer), answer.name());
        }
        if let Adding::Refused(why) = item.add {
            let why = format!("can't add: {why}");
            println!("  {}", style.paint(why, &[Paint::Dim]));
        }
        let always: Vec<String> = Answer::ALWAYS
            .iter()
            .map(|answer| format!("{} {}", key(*answer), answer.name()))
            .collect();
        println!("  {}", always.join(" · "));
        let keys: Vec<(char, Answer)> = choices
            .iter()
            .map(|(answer, _)| *answer)
            .chain(Answer::ALWAYS)
            .map(|answer| (answer.key(), answer))
            .collect();

        loop {
            let answer = match prompt.ask("  ›", &keys) {
                Ok(Some(answer)) => answer,
                Ok(None) => break 'items,
                Err(e) if prompt::stopped(&e) => break 'items,
                Err(e) => {
                    failure = Some(e);
                    break 'items;
                }
            };
            let outcome = match answer {
                Answer::Quit => break 'items,
                Answer::Add(add) | Answer::Secret(add) => add
                    .apply(fleet, member, prompt, &item.seen)
                    .map(|added| added.then_some("added to the fleet")),
                Answer::NotSecret => edit::not_secret(fleet, member, &item.subject)
                    .map(|()| Some("marked not secret")),
                Answer::Revert(revert) => {
                    revert_one(fleet, &mut record, (item.member, revert), prompt)
                        .map(|reverted| reverted.then_some("reverted"))
                }
                Answer::Ignore => {
                    edit::ignore(fleet, member, &item.ignore_line()).map(|()| Some("ignored"))
                }
                Answer::Later => Ok(Some(back_later.as_str())),
                Answer::Pass => Ok(Some("passed")),
            };
            let done = match outcome {
                Ok(Some(done)) => done,
                Ok(None) => continue,
                Err(e) if prompt::stopped(&e) => break 'items,
                Err(e) => {
                    println!("  failed: {}", style::clean(&format!("{e:#}")));
                    continue;
                }
            };
            match answer {
                Answer::Later => later.put_off(fleet, item, now),
                Answer::Pass => {}
                _ => later.remove(fleet, item),
            }
            tally.count(answer);
            last = Some(format!("{}: {done}", style::clean(&item.subject)));
            break;
        }
        seen = n + 1;
    }

    later.save(fleet, &mine)?;
    if let Some(e) = failure {
        return Err(e);
    }
    if style.terminal {
        print!("{}", style::CLEAR);
    }
    if due.is_empty() {
        println!("Nothing to review.");
    } else {
        summary(&tally, style);
    }
    let unseen = due.len() - seen;
    let notes = [
        (unseen, "not seen yet, for next time"),
        (mine.len() - due.len(), "put off before and not due yet"),
    ];
    for (count, what) in notes {
        if count > 0 {
            println!("{}", style.paint(format!("{count} {what}"), &[Paint::Dim]));
        }
    }
    let from_fleet = report.shown.len() - mine.len();
    if from_fleet > 0 {
        let what = format!("{from_fleet} coming from the fleet, for");
        println!(
            "{} {}",
            style.paint(what, &[Paint::Dim]),
            style.command("apply")
        );
    }
    if tally.not_secret > 0 {
        println!(
            "{} marked not secret, which the next review shows as ordinary files",
            tally.not_secret
        );
    }
    Ok(tally)
}

fn choices<'a>(
    item: &'a Item,
    add: Option<&'a Add>,
    encrypted: Option<&'a Add>,
) -> Vec<(Answer<'a>, String)> {
    let mut choices = Vec::new();
    if let Some(add) = add {
        choices.push((Answer::Add(add), add.describe()));
    }
    if let Some(encrypted) = encrypted {
        choices.push((Answer::Secret(encrypted), encrypted.describe()));
    }
    if item.secret == Secret::Guessed {
        let what = "list it in not-secret.txt, and add it later in clear";
        choices.push((Answer::NotSecret, what.into()));
    }
    if let Some(revert) = &item.revert {
        choices.push((Answer::Revert(revert), revert.describe()));
    }
    choices
}

fn summary(tally: &Tally, style: Style) {
    let parts: Vec<String> = tally
        .counts()
        .iter()
        .filter(|(count, ..)| *count > 0)
        .map(|(count, what, paint)| style.paint(format!("{count} {what}"), &[*paint]))
        .collect();
    if !parts.is_empty() {
        println!("{}", parts.join(", "));
    }
}

fn progress(done: usize, total: usize, style: Style) -> String {
    const WIDTH: usize = 20;
    let filled = (done * WIDTH).div_ceil(total.max(1));
    format!(
        "{}{}",
        style.paint("━".repeat(filled), &[Paint::Cyan]),
        style.paint("━".repeat(WIDTH - filled), &[Paint::Dim])
    )
}

fn revert_one(
    fleet: &Fleet,
    record: &mut Record,
    (member, revert): revert::Owned,
    prompt: &mut Prompt,
) -> Result<bool> {
    if let Revert::Machine(Machine::Etc(Etc::Delete(path))) = revert
        && path.is_dir()
    {
        let count = walk::files(path)?.len();
        let files = style::count(count, "file");
        if !prompt.confirm(&format!("  That deletes {files}. Go on"))? {
            return Ok(false);
        }
    }
    revert::run(fleet, &[(member, revert)])?;
    record.reverted(revert)?;
    record.save()?;
    Ok(true)
}
