mod fleet;
mod home;
mod install;
mod package;
mod probe;
mod secrets;
mod services;
#[cfg(test)]
#[path = "../tests/support.rs"]
mod support;
mod sync;
mod system;
mod trust;
mod usage;
mod util;

use anyhow::bail;
use fleet::{
    Fleet,
    checked::{self, Advance, Outcome, Remote},
};
use std::process::ExitCode;
use util::prompt::Prompt;

#[derive(Clone, Copy)]
enum Command {
    Scan,
    Review,
    Apply,
    Recovery,
    Commit,
    Trust,
    Retire,
    Install,
    Secret,
}

impl Command {
    fn parse(name: &str) -> Option<Command> {
        Some(match name {
            "scan" => Command::Scan,
            "review" => Command::Review,
            "apply" => Command::Apply,
            "recovery" => Command::Recovery,
            "commit" => Command::Commit,
            "trust" => Command::Trust,
            "retire" => Command::Retire,
            "install" => Command::Install,
            "secret" => Command::Secret,
            _ => return None,
        })
    }
}

fn main() -> ExitCode {
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let help = |arg: &String| matches!(arg.as_str(), "--help" | "-h" | "help");
    match &args[..] {
        [] => {
            println!("{}", usage::text());
            return ExitCode::SUCCESS;
        }
        [arg] if help(arg) => {
            println!("{}", usage::text());
            return ExitCode::SUCCESS;
        }
        [install, arg] if install == "install" && help(arg) => {
            println!("{}", usage::install());
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    let result = match &args[..] {
        [probe] if probe == "probe" => probe::run(),
        [start, interface @ ..] if start == "start" && interface.len() <= 1 => {
            start_services(interface.first().map(String::as_str))
        }
        _ => run(&args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let text = format!("{e:#}");
            let lines: Vec<String> = text.lines().map(util::style::clean).collect();
            eprintln!("Error: {}", lines.join("\n"));
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> anyhow::Result<()> {
    let (name, rest) = args.split_first().expect("main handles no arguments");
    let Some(command) = Command::parse(name) else {
        bail!("mothership has no command {name}: `mothership --help` lists them");
    };
    if !system::user::is_root() {
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
            "This machine refuses {} \"{}\" of {}, which claims to be by {}, and what follows \
             it: {}",
            checked::short(&commit.commit),
            commit.subject,
            commit.date,
            commit.author,
            commit.why
        );
    }
    match (command, rest) {
        (Command::Scan, []) => sync::scan::run(refused),
        (Command::Review, []) => sync::review::run(),
        (Command::Apply, []) => sync::apply::run(),
        (Command::Install, flags) => install::run(flags),
        (Command::Recovery, []) => {
            trust::recovery::run(std::path::Path::new(fleet::DIR), &mut Prompt::new())
        }
        (Command::Commit, [message]) => {
            let mut prompt = Prompt::new();
            let admin = fleet::admin()?;
            let stage = checked::stage()?;
            stage.take_checkout(&admin)?;
            let key = trust::sign::require_machine_key()?;
            let outcome = stage.commit(&admin, message, &key, &mut prompt, Advance::KeepEdits)?;
            if let Outcome::Unchanged = outcome {
                println!("Nothing to commit.");
            }
            Ok(())
        }
        (Command::Trust, [host, type_name, keys @ ..]) if !keys.is_empty() => {
            system::host::check(host)?;
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
            trust::github::add(&fleet::admin()?, &uploads)?;
            println!("On {host}, type this code: {}", checked::short(&code));
            Ok(())
        }
        (Command::Retire, [host]) => {
            system::host::check(host)?;
            let (outcome, _) = edit_fleet(&format!("Retire {host}"), |fleet, _| {
                trust::retire_host(fleet, host)
            })?;
            match outcome {
                Outcome::Declined => Ok(()),
                Outcome::Unchanged | Outcome::Pushed(_) => {
                    trust::github::remove(&fleet::admin()?, host)
                }
            }
        }
        (Command::Secret, [service, name]) => {
            let machine = fleet::Machine::load()?;
            let wanted = fleet::edit::secret::Wanted::parse(service, name, &machine.device_type)?;
            let message = format!("Set {} of {}", wanted.name, wanted.shown());
            edit_fleet(&message, |fleet, prompt| {
                fleet::edit::secret::set(fleet, &wanted, prompt)
            })
            .map(drop)
        }
        _ => bail!("usage: {}", usage::of(name)),
    }
}

fn edit_fleet(
    message: &str,
    command: impl FnOnce(&Fleet, &mut Prompt) -> anyhow::Result<()>,
) -> anyhow::Result<(Outcome, String)> {
    let key = trust::sign::require_machine_key()?;
    let mut prompt = Prompt::new();
    let stage = checked::stage()?;
    let base = stage.base().to_owned();
    let fleet = Fleet::load_at(&stage.dir)?;
    command(&fleet, &mut prompt)?;
    let outcome = stage.commit(fleet.admin(), message, &key, &mut prompt, Advance::Merge)?;
    Ok((outcome, base))
}

fn start_services(interface: Option<&str>) -> anyhow::Result<()> {
    if !system::user::is_root() {
        bail!("mothership start runs as root, from services' units");
    }
    if let Some(interface) = interface
        && services::Network::try_from(interface.to_owned())?
            .interface()
            .is_none()
    {
        bail!("{interface} isn't a network interface");
    }
    if let Some(interface) = interface {
        services::start::wait_for_address(interface)?;
    }
    match lock()? {
        Some(_lock) => services::start::command(interface),
        None => {
            println!(
                "Another mothership command is running: scan shows a service this leaves stopped"
            );
            Ok(())
        }
    }
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
