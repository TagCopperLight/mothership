use crate::{
    edit::{self, Add},
    fleet::Fleet,
    later::{self, DAY, Later},
    prompt::Prompt,
    record::{self, Record},
    revert::{self, Revert},
    scan::{self, Columns, Item, Secret, Section},
    style::{self, Paint, Style},
    walk,
};
use anyhow::Result;
use std::path::Path;

pub fn run() -> Result<()> {
    let report = scan::collect()?;
    let result = ask(&report);
    report.fleet.hand_back()?;
    result
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
    fn count(&mut self, answer: Answer) {
        let count = match answer {
            Answer::Add | Answer::Secret => &mut self.added,
            Answer::NotSecret => &mut self.not_secret,
            Answer::Revert => &mut self.reverted,
            Answer::Ignore => &mut self.ignored,
            Answer::Later => &mut self.put_off,
            Answer::Pass => &mut self.passed,
            Answer::Quit => return,
        };
        *count += 1;
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Answer {
    Add,
    Secret,
    NotSecret,
    Revert,
    Ignore,
    Later,
    Pass,
    Quit,
}

impl Answer {
    /// Offered for every item, so review lists them on one line.
    const ALWAYS: [Answer; 4] = [Answer::Ignore, Answer::Later, Answer::Pass, Answer::Quit];
    const ALL: [Answer; 8] = [
        Answer::Add,
        Answer::Secret,
        Answer::NotSecret,
        Answer::Revert,
        Answer::Ignore,
        Answer::Later,
        Answer::Pass,
        Answer::Quit,
    ];

    fn key(self) -> char {
        match self {
            Answer::Add => 'a',
            Answer::Secret => 's',
            Answer::NotSecret => 'n',
            Answer::Revert => 'r',
            Answer::Ignore => 'i',
            Answer::Later => 'l',
            Answer::Pass => 'p',
            Answer::Quit => 'q',
        }
    }

    fn name(self) -> &'static str {
        match self {
            Answer::Add => "add",
            Answer::Secret => "secret",
            Answer::NotSecret => "not secret",
            Answer::Revert => "revert",
            Answer::Ignore => "ignore",
            Answer::Later => "later",
            Answer::Pass => "pass",
            Answer::Quit => "quit",
        }
    }

    fn from_key(key: char) -> Answer {
        Answer::ALL
            .into_iter()
            .find(|answer| answer.key() == key)
            .expect("the prompt accepts only offered keys")
    }
}

fn ask(report: &scan::Report) -> Result<()> {
    let fleet = &report.fleet;
    let style = Style::stdout();
    let mut later = Later::load()?;
    let mut record = Record::load(Path::new(record::PATH))?;
    let now = later::now()?;
    // The fleet's changes are apply's to follow, not mine to sort.
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

    let mut prompt = Prompt::new();
    let mut tally = Tally::default();
    let mut last: Option<String> = None;
    let mut seen = 0;
    'items: for (n, item) in due.iter().enumerate() {
        if style.terminal {
            print!("\x1b[2J\x1b[3J\x1b[H");
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
        item.preview.show(style.color);

        let add = item.add.as_ref();
        let encrypted = add
            .filter(|_| item.secret == Secret::No)
            .and_then(Add::encrypted);
        let revert = item.revert.as_ref();
        let mut choices: Vec<(Answer, String)> = Vec::new();
        if let Some(add) = add {
            choices.push((Answer::Add, add.describe()));
        }
        if let Some(encrypted) = &encrypted {
            choices.push((Answer::Secret, encrypted.describe()));
        }
        if item.secret == Secret::Guessed {
            let what = "list it in not-secret.txt, and add it later in clear";
            choices.push((Answer::NotSecret, what.into()));
        }
        if let Some(revert) = revert {
            choices.push((Answer::Revert, revert.describe()));
        }
        let key = |answer: Answer| style.paint(answer.key(), &[Paint::Bold, Paint::Cyan]);
        for (answer, what) in &choices {
            println!("  {}  {:10}  {what}", key(*answer), answer.name());
        }
        if let Some(why) = item.no_add {
            let why = format!("can't add: {why}");
            println!("  {}", style.paint(why, &[Paint::Dim]));
        }
        let always: Vec<String> = Answer::ALWAYS
            .iter()
            .map(|answer| format!("{} {}", key(*answer), answer.name()))
            .collect();
        println!("  {}", always.join(" · "));
        let keys: Vec<char> = choices
            .iter()
            .map(|(answer, _)| *answer)
            .chain(Answer::ALWAYS)
            .map(Answer::key)
            .collect();

        loop {
            let Some(answer) = prompt.ask("  ›", &keys)?.map(Answer::from_key) else {
                break 'items;
            };
            let outcome = match answer {
                Answer::Quit => break 'items,
                Answer::Add | Answer::Secret => {
                    let add = match answer {
                        Answer::Add => add,
                        _ => encrypted.as_ref(),
                    };
                    let add = add.expect("offered only when there is one");
                    add.apply(fleet, member, &mut prompt)
                        .map(|added| added.then_some("added to the fleet"))
                }
                Answer::NotSecret => edit::not_secret(fleet, member, &item.subject)
                    .map(|()| Some("marked not secret")),
                Answer::Revert => {
                    let revert = (item.member, revert.expect("offered only when there is one"));
                    revert_one(fleet, &mut record, revert, &mut prompt)
                        .map(|reverted| reverted.then_some("reverted"))
                }
                Answer::Ignore => {
                    edit::ignore(fleet, member, &scan::ignore_line(item)).map(|()| Some("ignored"))
                }
                Answer::Later => Ok(Some("back in 7 days")),
                Answer::Pass => Ok(Some("passed")),
            };
            // A failed or cancelled answer asks again about the same item.
            let done = match outcome {
                Ok(Some(done)) => done,
                Ok(None) => continue,
                Err(e) => {
                    println!("  failed: {e:#}");
                    continue;
                }
            };
            match answer {
                Answer::Later => later.put_off(fleet, item, now),
                Answer::Pass => {}
                _ => later.remove(fleet, item),
            }
            tally.count(answer);
            last = Some(format!("{}: {done}", item.subject));
            break;
        }
        seen = n + 1;
    }

    later.save(fleet, &mine)?;
    if style.terminal {
        print!("\x1b[2J\x1b[3J\x1b[H");
    }
    if due.is_empty() {
        println!("Nothing to review.");
    } else {
        summary(fleet, &tally, style);
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
    if tally.added + tally.ignored + tally.not_secret > 0 {
        let diff = format!("git -C {} diff", fleet.dir.display());
        println!(
            "Check the changes with {}, then commit.",
            style.paint(diff, &[Paint::Cyan])
        );
    }
    Ok(())
}

fn summary(fleet: &Fleet, tally: &Tally, style: Style) {
    let counts = [
        (tally.added, "added", Paint::Green),
        (tally.reverted, "reverted", Paint::Yellow),
        (tally.ignored, "ignored", Paint::Dim),
        (tally.put_off, "put off for later", Paint::Blue),
        (tally.passed, "passed", Paint::Dim),
    ];
    let parts: Vec<String> = counts
        .iter()
        .filter(|(count, ..)| *count > 0)
        .map(|(count, what, paint)| style.paint(format!("{count} {what}"), &[*paint]))
        .collect();
    if parts.is_empty() && tally.not_secret == 0 {
        println!("Nothing changed in {}.", fleet.dir.display());
    } else if !parts.is_empty() {
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
    if let Revert::DeleteEtc(path) = revert
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
    record.save(Path::new(record::PATH))?;
    Ok(true)
}
