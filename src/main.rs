mod apply;
mod chezmoi;
mod cmd;
mod edit;
mod file;
mod fleet;
mod hardware;
mod hash;
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
mod revert;
mod review;
mod scan;
mod secrets;
mod snapshot;
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
use fleet::Fleet;
use prompt::Prompt;
use std::process::ExitCode;

const USAGE: &str = "usage: mothership scan | review | apply | key \
                     | trust <type>/<root|user> <name> <key> | retire <type>/<root|user> <name> \
                     | install [--fleet <repo>] [--type <type>] [--option <option>]... \
                     [--hostname <name>] [--admin <user>] [--timezone <zone>] [--disk <disk>] \
                     [--layout wipe|beside] [--remove-unlisted yes|no]";

#[derive(Clone, Copy)]
enum Command {
    Scan,
    Review,
    Apply,
    Key,
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
    let _lock = match command {
        Command::Scan => None,
        _ => {
            let lock = lock()?;
            package::aur::NoPassword::remove_leftover()?;
            Some(lock)
        }
    };
    if !matches!(command, Command::Install) {
        fleet::require_admin_sudo()?;
    }
    match (command, rest) {
        (Command::Scan, []) => scan::run(),
        (Command::Review, []) => review::run(),
        (Command::Apply, []) => apply::run(),
        (Command::Install, flags) => install::run(flags),
        (Command::Key, []) => trust::key(&Fleet::load()?),
        (Command::Trust, [target, name, key]) => {
            edit_fleet(|fleet| trust::trust(fleet, target, name, key, &mut Prompt::new()))
        }
        (Command::Retire, [target, name]) => {
            edit_fleet(|fleet| trust::retire(fleet, target, name, &mut Prompt::new()))
        }
        _ => usage(),
    }
}

fn usage() -> ! {
    eprintln!("{USAGE}");
    std::process::exit(2)
}

fn edit_fleet(command: impl FnOnce(&Fleet) -> anyhow::Result<()>) -> anyhow::Result<()> {
    let fleet = Fleet::load()?;
    let result = command(&fleet);
    result.and(fleet.hand_back())
}

fn lock() -> anyhow::Result<std::fs::File> {
    use anyhow::Context;
    use std::os::fd::AsRawFd;
    std::fs::create_dir_all(LOCK_DIR)?;
    let path = std::path::Path::new(LOCK_DIR).join("lock");
    let file =
        std::fs::File::create(&path).with_context(|| format!("opening {}", path.display()))?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        bail!("another mothership command is running");
    }
    Ok(file)
}

const LOCK_DIR: &str = "/var/lib/mothership";
