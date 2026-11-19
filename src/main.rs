mod apply;
mod chezmoi;
mod cmd;
mod edit;
mod fleet;
mod hardware;
mod ignore;
mod install;
mod later;
mod net;
mod package;
mod preview;
mod prompt;
mod record;
mod revert;
mod review;
mod scan;
mod secrets;
mod style;
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

fn main() -> anyhow::Result<()> {
    // Rust ignores SIGPIPE, so `mothership scan | head` would panic on the closed pipe.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !matches!(
        args.first().map(String::as_str),
        Some("scan" | "review" | "apply" | "key" | "trust" | "retire" | "install")
    ) {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }
    if !user::is_root() {
        bail!("mothership reads every user's home: run it with sudo");
    }
    match args.as_slice() {
        [command] if command == "scan" => scan::run(),
        [command] if command == "review" => review::run(),
        [command] if command == "apply" => apply::run(),
        [command, flags @ ..] if command == "install" => install::run(flags),
        [command] if command == "key" => secrets::key(&Fleet::load()?),
        [command, target, name, key] if command == "trust" => {
            edit_fleet(|fleet| secrets::trust(fleet, target, name, key))
        }
        [command, target, name] if command == "retire" => {
            edit_fleet(|fleet| secrets::retire(fleet, target, name))
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

fn edit_fleet(command: impl FnOnce(&Fleet) -> anyhow::Result<()>) -> anyhow::Result<()> {
    let fleet = Fleet::load()?;
    let result = command(&fleet);
    fleet.hand_back()?;
    result
}
