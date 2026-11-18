use crate::{
    edit,
    fleet::{Fleet, tilde},
    prompt::Prompt,
    record::{self, Record},
    revert::{self, Revert},
    scan::{self, Item},
    user, walk,
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

pub fn run() -> Result<()> {
    let report = scan::collect()?;
    let mut later = Later::load()?;
    // Only root can change the machine, and only root can save what it changed in the record.
    let mut record = if user::is_root() {
        Some(Record::load(Path::new(record::PATH))?)
    } else {
        None
    };
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
    'items: for (n, item) in due.iter().enumerate() {
        if terminal {
            print!("\x1b[2J\x1b[3J\x1b[H");
        }
        let secret = if item.secret { "  [secret]" } else { "" };
        println!("[{}/{}] {}", n + 1, due.len(), item.kind.title());
        println!("{bold}{}{plain}  {}{secret}", item.subject, item.detail);
        if let Some(waiting) = later.get(item) {
            println!("  waiting for {} days", (now - waiting.first) / DAY);
        }
        let add = if item.secret {
            println!("  secrets wait for the secrets system, so it can't be added yet");
            None
        } else {
            item.add.as_ref()
        };
        let revert = item.revert.as_ref().filter(|_| record.is_some());
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
        if revert.is_some() {
            choices.push("(r)evert");
        }
        choices.extend(["(i)gnore", "(l)ater", "(p)ass", "(q)uit"]);
        loop {
            match prompt.ask(&format!("  {}?", choices.join(" ")))?.as_deref() {
                None | Some("q") => break 'items,
                Some("a") if add.is_some() => {
                    let add = add.expect("checked");
                    if !done(add.apply(&report.fleet, &mut prompt)) {
                        continue;
                    }
                    later.remove(item);
                    added += 1;
                }
                Some("r") if revert.is_some() => {
                    let record = record
                        .as_mut()
                        .expect("revert is only offered with a record");
                    let revert = revert.expect("checked");
                    if !done(revert_one(&report.fleet, record, revert, &mut prompt)) {
                        continue;
                    }
                    later.remove(item);
                    reverted += 1;
                }
                Some("i") => {
                    edit::ignore(&report.fleet, &scan::ignore_line(item))?;
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
    if record.is_none() && due.iter().any(|item| item.revert.is_some()) {
        println!("Run it with sudo to also revert this machine to the fleet.");
    }
    if waiting > 0 {
        println!("{waiting} put off before and not due yet");
    }
    if added + ignored > 0 {
        println!(
            "Check the changes with `git -C {} diff`, then commit.",
            tilde(&report.fleet.dir)
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
    revert: &Revert,
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
    revert::run(fleet, &[revert])?;
    record.reverted(revert)?;
    record.save(Path::new(record::PATH))?;
    Ok(true)
}

#[derive(Serialize, Deserialize, Clone, Copy)]
struct Waiting {
    first: u64,
    last: u64,
}

struct Later {
    path: PathBuf,
    items: BTreeMap<String, Waiting>,
}

impl Later {
    fn load() -> Result<Self> {
        let path = user::home().join(".local/state/mothership/later.json");
        let items = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("parsing {}", path.display()))?,
            Err(e) if e.kind() == ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Ok(Later { path, items })
    }

    fn get(&self, item: &Item) -> Option<Waiting> {
        self.items.get(&scan::ignore_line(item)).copied()
    }

    fn due(&self, item: &Item, now: u64) -> bool {
        self.get(item).is_none_or(|w| now - w.last >= LATER)
    }

    fn put_off(&mut self, item: &Item, now: u64) {
        let waiting = self
            .items
            .entry(scan::ignore_line(item))
            .or_insert(Waiting {
                first: now,
                last: now,
            });
        waiting.last = now;
    }

    fn remove(&mut self, item: &Item) {
        self.items.remove(&scan::ignore_line(item));
    }

    fn save(mut self, current: &[Item]) -> Result<()> {
        let keys: Vec<String> = current.iter().map(scan::ignore_line).collect();
        self.items.retain(|key, _| keys.contains(key));
        user::create_dir_all(self.path.parent().expect("has a parent"))?;
        fs::write(&self.path, serde_json::to_string_pretty(&self.items)?)
            .with_context(|| format!("writing {}", self.path.display()))?;
        user::own(&self.path)
    }
}

#[cfg(test)]
#[path = "../tests/review.rs"]
mod tests;
