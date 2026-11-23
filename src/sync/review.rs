use crate::{
    fleet::edit::{self, Add},
    fleet::{
        Fleet,
        checked::{self, Advance, Stage},
    },
    sync::item::{Adding, Item, Secret, Section},
    sync::later::{self, DAY, Later},
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
    let mut stage = checked::stage()?;
    let report = scan::collect(&stage.dir)?;
    let signer = stage.machine_signer()?;
    let key = sign::require_machine_key()?;
    let mut prompt = Prompt::new();
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
        answers.later.save(fleet, &answers.mine)?;
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
    answers.later.save(fleet, &answers.mine)?;
    revert_all(fleet, &answers.reverts)?;
    stage.finish()
}

struct Answers<'a> {
    tally: Tally,
    later: Later,
    mine: Vec<&'a Item>,
    reverts: Vec<(String, revert::Owned<'a>)>,
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
            Answer::Revert => &mut self.reverted,
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
    Revert,
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
            Answer::Revert => 'r',
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
            Answer::Revert => "revert",
            Answer::Ignore => "ignore",
            Answer::Later => "later",
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
    let mut later = Later::load()?;
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
    let mut reverts = Vec::new();
    let mut last: Option<String> = None;
    let mut seen = 0;
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
                Answer::NotSecret => edit::not_secret(fleet, member, &item.subject)
                    .map(|()| Some("marked not secret")),
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
                Answer::Later => Ok(Some(back_later.as_str())),
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
    Ok(Answers {
        tally,
        later,
        mine,
        reverts,
    })
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
