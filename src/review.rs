use crate::{
    edit::{self, Add},
    fleet::Fleet,
    prompt::Prompt,
    record::{self, Record},
    revert::{self, Revert},
    scan::{self, Item, Secret},
    walk,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{ErrorKind, IsTerminal},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const DAY: u64 = 24 * 60 * 60;
const LATER: u64 = 7 * DAY;

/// What review writes into the checkout goes back to the admin, even when an answer fails.
pub fn run() -> Result<()> {
    let report = scan::collect()?;
    let result = ask(&report);
    report.fleet.hand_back()?;
    result
}

fn ask(report: &scan::Report) -> Result<()> {
    let fleet = &report.fleet;
    let mut later = Later::load(fleet)?;
    let mut record = Record::load(Path::new(record::PATH))?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let due: Vec<&Item> = report
        .shown
        .iter()
        .filter(|item| later.due(item, now))
        .collect();
    let waiting = report.shown.len() - due.len();
    let terminal = std::io::stdout().is_terminal();
    let (bold, plain) = if terminal {
        ("\x1b[1m", "\x1b[0m")
    } else {
        ("", "")
    };

    let mut prompt = Prompt::new();
    let (mut added, mut reverted, mut ignored, mut put_off, mut passed) = (0, 0, 0, 0, 0);
    let mut not_secret = 0;
    'items: for (n, item) in due.iter().enumerate() {
        if terminal {
            print!("\x1b[2J\x1b[3J\x1b[H");
        }
        let member = item.member.map(|i| &fleet.members[i]);
        println!("[{}/{}] {}", n + 1, due.len(), scan::title(fleet, item));
        println!(
            "{bold}{}{plain}  {}{}",
            item.subject,
            item.detail,
            item.secret.mark()
        );
        if let Some(waiting) = later.get(item) {
            println!("  waiting for {} days", (now - waiting.first) / DAY);
        }
        if let Some(why) = item.no_add {
            println!("  {why}, so it can't be added yet");
        }
        let add = item.add.as_ref();
        // For a secret the scan missed.
        let encrypted = add
            .filter(|_| item.secret == Secret::No)
            .and_then(Add::encrypted);
        let guessed = item.secret == Secret::Guessed;
        let revert = item.revert.as_ref();
        item.preview.show(terminal);
        if let Some(add) = add {
            println!("  add: {}", add.describe());
        }
        if let Some(revert) = revert {
            println!("  revert: {}", revert.describe());
        }
        let mut choices = Vec::new();
        if add.is_some() {
            choices.push("(a)dd");
        }
        if encrypted.is_some() {
            choices.push("(s)ecret");
        }
        if guessed {
            choices.push("(n)ot secret");
        }
        if revert.is_some() {
            choices.push("(r)evert");
        }
        choices.extend(["(i)gnore", "(l)ater", "(p)ass", "(q)uit"]);
        loop {
            match prompt.ask(&format!("  {}?", choices.join(" ")))?.as_deref() {
                None | Some("q") => break 'items,
                Some(answer @ ("a" | "s")) => {
                    let add = match answer {
                        "a" => add,
                        _ => encrypted.as_ref(),
                    };
                    let Some(add) = add else { continue };
                    if !done(add.apply(fleet, member, &mut prompt)) {
                        continue;
                    }
                    later.remove(item);
                    added += 1;
                }
                Some("n") if guessed => {
                    edit::not_secret(fleet, member, &item.subject)?;
                    later.remove(item);
                    not_secret += 1;
                }
                Some("r") if revert.is_some() => {
                    let revert = (item.member, revert.expect("checked"));
                    if !done(revert_one(fleet, &mut record, revert, &mut prompt)) {
                        continue;
                    }
                    later.remove(item);
                    reverted += 1;
                }
                Some("i") => {
                    edit::ignore(fleet, member, &scan::ignore_line(item))?;
                    later.remove(item);
                    ignored += 1;
                }
                Some("l") => {
                    later.put_off(item, now);
                    put_off += 1;
                }
                Some("p") => passed += 1,
                _ => continue,
            }
            break;
        }
    }

    later.save(&report.shown)?;
    if terminal {
        print!("\x1b[2J\x1b[3J\x1b[H");
    }
    println!(
        "{added} added, {reverted} reverted, {ignored} ignored, {put_off} put off for later, \
         {passed} passed"
    );
    if waiting > 0 {
        println!("{waiting} put off before and not due yet");
    }
    if not_secret > 0 {
        println!("{not_secret} marked not secret, which the next review shows as ordinary files");
    }
    if added + ignored + not_secret > 0 {
        println!(
            "Check the changes with `git -C {} diff`, then commit.",
            fleet.dir.display()
        );
    }
    Ok(())
}

/// False when I cancelled or it failed, so the item is asked again.
fn done(result: Result<bool>) -> bool {
    result.unwrap_or_else(|e| {
        println!("  failed: {e:#}");
        false
    })
}

/// Returns false when I cancel.
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
        if !prompt.confirm(&format!("  That deletes {count} files. Go on"))? {
            return Ok(false);
        }
    }
    revert::run(fleet, &[(member, revert)])?;
    record.reverted(revert)?;
    record.save(Path::new(record::PATH))?;
    Ok(true)
}

#[derive(Serialize, Deserialize, Clone, Copy)]
struct Waiting {
    first: u64,
    last: u64,
}

/// Keyed by `scan::key`.
struct Later<'a> {
    fleet: &'a Fleet,
    path: PathBuf,
    items: BTreeMap<String, Waiting>,
}

const LATER_PATH: &str = "/var/lib/mothership/later.json";

impl<'a> Later<'a> {
    fn load(fleet: &'a Fleet) -> Result<Self> {
        let path = PathBuf::from(LATER_PATH);
        let items = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("parsing {}", path.display()))?,
            Err(e) if e.kind() == ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Ok(Later { fleet, path, items })
    }

    fn get(&self, item: &Item) -> Option<Waiting> {
        self.items.get(&scan::key(self.fleet, item)).copied()
    }

    fn due(&self, item: &Item, now: u64) -> bool {
        self.get(item).is_none_or(|w| now - w.last >= LATER)
    }

    fn put_off(&mut self, item: &Item, now: u64) {
        let waiting = self
            .items
            .entry(scan::key(self.fleet, item))
            .or_insert(Waiting {
                first: now,
                last: now,
            });
        waiting.last = now;
    }

    fn remove(&mut self, item: &Item) {
        self.items.remove(&scan::key(self.fleet, item));
    }

    fn save(mut self, current: &[Item]) -> Result<()> {
        let keys: Vec<String> = current.iter().map(|i| scan::key(self.fleet, i)).collect();
        self.items.retain(|key, _| keys.contains(key));
        fs::create_dir_all(self.path.parent().expect("has a parent"))?;
        fs::write(&self.path, serde_json::to_string_pretty(&self.items)?)
            .with_context(|| format!("writing {}", self.path.display()))
    }
}

#[cfg(test)]
#[path = "../tests/review.rs"]
mod tests;
