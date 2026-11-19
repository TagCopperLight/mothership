mod apply;
mod chezmoi;
mod cmd;
mod edit;
mod fleet;
mod hardware;
mod homes;
mod ignore;
mod install;
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
mod systemd;
mod tools;
mod user;
mod walk;

use anyhow::bail;
use fleet::Fleet;

const USAGE: &str = "usage: mothership scan | review | apply | key \
                     | trust <type>/<root|user> <name> <key> | retire <type>/<root|user> <name> \
                     | install [--fleet <repo>] [--type <type>] [--option <option>]... \
                     [--hostname <name>] [--admin <user>] [--timezone <zone>] [--disk <disk>] \
                     [--layout wipe|beside]";

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

fn main() -> anyhow::Result<()> {
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let args: Vec<String> = std::env::args().skip(1).collect();
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
        _ => Some(lock()?),
    };
    match (command, rest) {
        (Command::Scan, []) => scan::run(),
        (Command::Review, []) => review::run(),
        (Command::Apply, []) => apply::run(),
        (Command::Install, flags) => install::run(flags),
        (Command::Key, []) => secrets::key(&Fleet::load()?),
        (Command::Trust, [target, name, key]) => {
            edit_fleet(|fleet| secrets::trust(fleet, target, name, key))
        }
        (Command::Retire, [target, name]) => {
            edit_fleet(|fleet| secrets::retire(fleet, target, name))
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
