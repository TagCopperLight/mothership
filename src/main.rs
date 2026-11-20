mod apply;
mod chezmoi;
mod cmd;
mod edit;
mod file;
mod fleet;
mod github;
mod hardware;
mod hash;
mod host;
mod ignore;
mod install;
mod item;
mod kind;
mod later;
mod package;
mod parallel;
mod pref;
mod preview;
mod prompt;
mod record;
mod recovery;
mod revert;
mod review;
mod scan;
mod secrets;
mod sign;
mod snapshot;
mod ssh;
mod style;
#[cfg(test)]
#[path = "../tests/support.rs"]
mod support;
mod systemd;
mod tools;
mod trust;
mod update;
mod user;
mod walk;

use anyhow::bail;
use fleet::{
    Fleet,
    checked::{self, Advance, Outcome, Remote},
};
use prompt::Prompt;
use std::process::ExitCode;

const USAGE: &str = "usage: mothership scan | review | apply | commit <message> | key | recovery \
                     | trust <host> <type> <name>=<key>... | retire <host> \
                     | install [--fleet <repo>] [--type <type>] [--option <option>]... \
                     [--hostname <name>] [--admin <user>] [--timezone <zone>] [--disk <disk>] \
                     [--layout wipe|beside] [--remove-unlisted yes|no]";

#[derive(Clone, Copy)]
enum Command {
    Scan,
    Review,
    Apply,
    Key,
    Recovery,
    Commit,
    Trust,
    Retire,
    Install,
}

impl Command {
    fn parse(name: &str) -> Option<Command> {
        Some(match name {
            "scan" => Command::Scan,
            "review" => Command::Review,
            "apply" => Command::Apply,
            "key" => Command::Key,
            "recovery" => Command::Recovery,
            "commit" => Command::Commit,
            "trust" => Command::Trust,
            "retire" => Command::Retire,
            "install" => Command::Install,
            _ => return None,
        })
    }
}

fn main() -> ExitCode {
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [help] = &args[..]
        && matches!(help.as_str(), "--help" | "-h" | "help")
    {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let text = format!("{e:#}");
            let lines: Vec<String> = text.lines().map(style::clean).collect();
            eprintln!("Error: {}", lines.join("\n"));
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> anyhow::Result<()> {
    let Some((command, rest)) = args
        .split_first()
        .and_then(|(name, rest)| Some((Command::parse(name)?, rest)))
    else {
        usage()
    };
    if !user::is_root() {
        bail!("mothership reads every user's home: run it with sudo");
    }
    let lock = lock()?;
    if lock.is_none() && !matches!(command, Command::Scan) {
        bail!("another mothership command is running");
    }
    if lock.is_some() {
        package::aur::NoPassword::remove_leftover()?;
    }
    if !matches!(command, Command::Install) {
        fleet::require_admin_sudo()?;
    }
    let remote = match command {
        Command::Apply | Command::Review | Command::Commit => Some(Remote::Fetch),
        Command::Install | Command::Recovery => None,
        Command::Scan if lock.is_none() => {
            println!(
                "Another mothership command is running, so this shows the fleet as last checked."
            );
            None
        }
        _ => Some(Remote::Skip),
    };
    let refused = match remote {
        Some(remote) => checked::sync(&fleet::admin()?, remote)?.refused,
        None => None,
    };
    if let Some(commit) = &refused
        && !matches!(command, Command::Scan)
    {
        println!(
            "This machine refuses {} \"{}\" by {}, and what follows it: {}",
            checked::short(&commit.commit),
            commit.subject,
            commit.author,
            commit.why
        );
    }
    match (command, rest) {
        (Command::Scan, []) => scan::run(refused),
        (Command::Review, []) => review::run(),
        (Command::Apply, []) => apply::run(),
        (Command::Install, flags) => install::run(flags),
        (Command::Key, []) => trust::key(&Fleet::load()?),
        (Command::Recovery, []) => {
            recovery::run(std::path::Path::new(fleet::DIR), &mut Prompt::new())
        }
        (Command::Commit, [message]) => {
            let mut prompt = Prompt::new();
            let admin = fleet::admin()?;
            let stage = checked::stage()?;
            stage.take_checkout(&admin)?;
            let key = sign::require_machine_key()?;
            let outcome = stage.commit(&admin, message, &key, &mut prompt, Advance::KeepEdits)?;
            if let Outcome::Unchanged = outcome {
                println!("Nothing to commit.");
            }
            Ok(())
        }
        (Command::Trust, [host, type_name, keys @ ..]) if !keys.is_empty() => {
            let message = format!("Trust {host} as {type_name}");
            let mut uploads = Vec::new();
            let (outcome, base) = edit_fleet(&message, |fleet, prompt| {
                uploads = trust::trust_host(fleet, fleet.keys(), host, type_name, keys, prompt)?;
                Ok(())
            })?;
            let code = match outcome {
                Outcome::Declined => return Ok(()),
                Outcome::Unchanged => base,
                Outcome::Pushed(commit) => commit,
            };
            github::add(&fleet::admin()?, &uploads)?;
            println!("On {host}, type this code: {}", checked::short(&code));
            Ok(())
        }
        (Command::Retire, [host]) => {
            let (outcome, _) = edit_fleet(&format!("Retire {host}"), |fleet, prompt| {
                trust::retire_host(fleet, host, prompt)
            })?;
            match outcome {
                Outcome::Declined => Ok(()),
                Outcome::Unchanged | Outcome::Pushed(_) => github::remove(&fleet::admin()?, host),
            }
        }
        _ => usage(),
    }
}

fn usage() -> ! {
    eprintln!("{USAGE}");
    std::process::exit(2)
}

fn edit_fleet(
    message: &str,
    command: impl FnOnce(&Fleet, &mut Prompt) -> anyhow::Result<()>,
) -> anyhow::Result<(Outcome, String)> {
    let key = sign::require_machine_key()?;
    let mut prompt = Prompt::new();
    let stage = checked::stage()?;
    let base = stage.base().to_owned();
    let fleet = Fleet::load_at(&stage.dir)?;
    command(&fleet, &mut prompt)?;
    let outcome = stage.commit(fleet.admin(), message, &key, &mut prompt, Advance::Merge)?;
    Ok((outcome, base))
}

fn lock() -> anyhow::Result<Option<std::fs::File>> {
    use anyhow::Context;
    use std::os::fd::AsRawFd;
    std::fs::create_dir_all(LOCK_DIR)?;
    let path = std::path::Path::new(LOCK_DIR).join("lock");
    let file =
        std::fs::File::create(&path).with_context(|| format!("opening {}", path.display()))?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Ok(None);
    }
    Ok(Some(file))
}

const LOCK_DIR: &str = "/var/lib/mothership";
