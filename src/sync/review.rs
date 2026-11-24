use crate::{
    fleet::edit::{self, Add},
    fleet::{
        Fleet,
        checked::{self, Advance, Stage},
    },
    sync::item::{Adding, Item, Secret, Section},
    sync::record::Record,
    sync::revert::{self, Etc, Machine, Revert},
    sync::scan::{self, Columns},
    system::host,
    trust::sign::{self, Signer},
    util::prompt::{self, Prompt},
    util::style::{self, Paint, Style},
    util::walk,
};
use anyhow::Result;

pub fn run() -> Result<()> {
    let mut prompt = Prompt::new();
    let Some(mut stage) = checked::stage(&mut prompt)? else {
        return Ok(());
    };
    let report = scan::collect(&stage.dir)?;
    let signer = stage.machine_signer()?;
    let key = sign::require_machine_key()?;
    let answers = match ask(&report, &mut stage, signer.as_ref(), &mut prompt) {
        Err(e) if prompt::stopped(&e) => {
            println!("Nothing changed.");
            return stage.finish();
        }
        answers => answers?,
    };
    let fleet = &report.fleet;
    let message = match answers.tally.parts() {
        parts if parts.is_empty() => format!("Review on {}", host::name()?),
        parts => format!("Review on {}: {}", host::name()?, parts.join(", ")),
    };
    let prepared = stage.prepare(fleet.admin(), &message, &key)?;
    if prepared.is_none() && answers.reverts.is_empty() {
        return stage.finish();
    }
    if let Some(prepared) = &prepared {
        prepared.show()?;
    }
    if !answers.reverts.is_empty() {
        println!("Then on this machine:");
        for (subject, revert) in &answers.reverts {
            println!("  {}: {}", subject, revert.1.describe());
        }
    }
    let question = match (&prepared, answers.reverts.is_empty()) {
        (Some(_), true) => "Commit and push",
        (Some(_), false) => "Commit, push and revert",
        (None, _) => "Revert them",
    };
    if !prompt.confirm(question)? {
        println!("Nothing changed.");
        return stage.finish();
    }
    if let Some(prepared) = prepared {
        prepared.publish(fleet.admin(), Advance::Merge)?;
    }
    revert_all(fleet, &answers.reverts)?;
    stage.finish()
}

struct Answers<'a> {
    tally: Tally,
    reverts: Vec<(String, revert::Owned<'a>)>,
}

#[derive(Default)]
struct Tally {
    added: usize,
    reverted: usize,
    ignored: usize,
    passed: usize,
}

impl Tally {
    fn counts(&self) -> [(usize, &'static str, Paint); 4] {
        [
            (self.added, "added", Paint::Green),
            (self.reverted, "reverted", Paint::Yellow),
            (self.ignored, "ignored", Paint::Dim),
            (self.passed, "passed", Paint::Dim),
        ]
    }

    fn parts(&self) -> Vec<String> {
        [(self.added, "added"), (self.ignored, "ignored")]
            .iter()
            .filter(|(count, _)| *count > 0)
            .map(|(count, what)| format!("{count} {what}"))
            .collect()
    }

    fn count(&mut self, answer: Answer) {
        let count = match answer {
            Answer::Add(_) | Answer::Secret(_) => &mut self.added,
            Answer::Revert => &mut self.reverted,
            Answer::Ignore => &mut self.ignored,
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
    Revert,
    Ignore,
    Pass,
    Quit,
}

impl Answer<'_> {
    const ALWAYS: [Answer<'static>; 3] = [Answer::Ignore, Answer::Pass, Answer::Quit];

    fn key(self) -> char {
        match self {
            Answer::Add(_) => 'a',
            Answer::Secret(_) => 's',
            Answer::Revert => 'r',
            Answer::Ignore => 'i',
            Answer::Pass => 'p',
            Answer::Quit => 'q',
        }
    }

    fn name(self) -> &'static str {
        match self {
            Answer::Add(_) => "add",
            Answer::Secret(_) => "secret",
            Answer::Revert => "revert",
            Answer::Ignore => "ignore",
            Answer::Pass => "pass",
            Answer::Quit => "quit",
        }
    }
}

fn ask<'a>(
    report: &'a scan::Report,
    stage: &mut Stage,
    signer: Option<&Signer>,
    prompt: &mut Prompt,
) -> Result<Answers<'a>> {
    let fleet = &report.fleet;
    let style = Style::stdout();
    let mine: Vec<&Item> = report
        .shown
        .iter()
        .filter(|item| item.section != Section::Fleet)
        .collect();

    let mut tally = Tally::default();
    let mut reverts = Vec::new();
    let mut last: Option<String> = None;
    let mut seen = 0;
    'items: for (n, item) in mine.iter().enumerate() {
        if style.terminal {
            print!("{}", style::CLEAR);
            if let Some(last) = last.take() {
                println!("{}\n", style.paint(last, &[Paint::Dim]));
            }
        }
        seen = n;
        let member = item.member.map(|i| &fleet.members[i]);
        let mut header = format!("[{}/{}] {}", n + 1, mine.len(), scan::heading(fleet, item));
        if style.terminal {
            header = format!(
                "{}  {}",
                style.paint(header, &[Paint::Bold]),
                progress(n, mine.len(), style)
            );
        }
        println!("{header}");
        println!("{}", scan::line(item, style, Columns::of(&[item])));
        item.preview.show(&item.seen, style.color);

        let add = match &item.add {
            Adding::Offered(add) => Some(add),
            Adding::Refused(_) | Adding::Never => None,
        };
        let encrypted = add
            .filter(|_| matches!(item.secret, Secret::No | Secret::Guessed))
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
        let mut keys: Vec<(char, Answer)> = choices
            .iter()
            .map(|(answer, _)| *answer)
            .chain(Answer::ALWAYS)
            .map(|answer| (answer.key(), answer))
            .collect();

        loop {
            let Some(answer) = prompt.ask("  ›", &keys)? else {
                break 'items;
            };
            let outcome = match answer {
                Answer::Quit => break 'items,
                Answer::Add(add) | Answer::Secret(add) => add
                    .apply(fleet, member, prompt, &item.seen)
                    .map(|added| added.then_some("added to the fleet")),
                Answer::Revert => {
                    let revert = item.revert.as_ref().expect("revert is offered with one");
                    confirm_revert(revert, prompt).map(|go| {
                        go.then(|| {
                            reverts.push((item.subject.clone(), (item.member, revert)));
                            "to revert"
                        })
                    })
                }
                Answer::Ignore => {
                    edit::ignore(fleet, member, &item.ignore_line()).map(|()| Some("ignored"))
                }
                Answer::Pass => Ok(Some("passed")),
            };
            let done = match outcome {
                Ok(Some(done)) => done,
                Ok(None) => continue,
                Err(e) if prompt::stopped(&e) => return Err(e),
                Err(e) => {
                    println!("  failed: {}", style::clean(&format!("{e:#}")));
                    continue;
                }
            };
            if let Some(why) = stage.keep_if_signable(signer)? {
                println!("  this machine can't sign that, so answer it from one that can: {why}");
                keys.retain(|(key, _)| *key != answer.key());
                continue;
            }
            tally.count(answer);
            last = Some(format!("{}: {done}", style::clean(&item.subject)));
            break;
        }
        seen = n + 1;
    }

    if style.terminal {
        print!("{}", style::CLEAR);
    }
    if mine.is_empty() {
        println!("Nothing to review.");
    } else {
        summary(&tally, style);
    }
    let unseen = mine.len() - seen;
    if unseen > 0 {
        let what = format!("{unseen} not seen yet, for next time");
        println!("{}", style.paint(what, &[Paint::Dim]));
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
    Ok(Answers { tally, reverts })
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
        let secret = (Answer::Secret(encrypted), encrypted.describe());
        match item.secret {
            Secret::Guessed => choices.insert(0, secret),
            _ => choices.push(secret),
        }
    }
    if let Some(revert) = &item.revert {
        choices.push((Answer::Revert, revert.describe()));
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

fn confirm_revert(revert: &Revert, prompt: &mut Prompt) -> Result<bool> {
    if let Revert::Machine(Machine::Etc(Etc::Delete(path))) = revert
        && path.is_dir()
    {
        let count = walk::files(path)?.len();
        let files = style::count(count, "file");
        return prompt.confirm(&format!("  That deletes {files}. Go on"));
    }
    Ok(true)
}

fn revert_all(fleet: &Fleet, reverts: &[(String, revert::Owned)]) -> Result<()> {
    let mut record = Record::load()?;
    for (subject, (member, revert)) in reverts {
        match revert::run(fleet, &[(*member, revert)]) {
            Ok(()) => record.reverted(revert)?,
            Err(e) => println!("Couldn't revert {subject}: {e:#}"),
        }
    }
    record.save()
}
