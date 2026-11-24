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

use anyhow::{Context, bail};
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
            let Some(stage) = checked::stage(&mut prompt)? else {
                return Ok(());
            };
            stage.take_checkout(&admin)?;
            let key = trust::sign::require_machine_key()?;
            let who = checked::author(&admin)?;
            let outcome =
                stage.commit(&admin, &who, message, &key, &mut prompt, Advance::KeepEdits)?;
            if let Outcome::Unchanged(_) = outcome {
                println!("Nothing to commit.");
            }
            Ok(())
        }
        (Command::Trust, [host, type_name, rest @ ..]) => {
            let (options, keys) = trust::options_and_keys(rest)?;
            if keys.is_empty() {
                bail!("usage: {}", usage::of(name));
            }
            system::host::check(host)?;
            let machine = fleet::Machine {
                device_type: type_name.clone(),
                options,
            };
            let message = format!("Trust {host} as {type_name}");
            let mut uploads = Vec::new();
            let outcome = edit_fleet(&message, |fleet, prompt| {
                uploads = trust::trust_host(fleet, fleet.keys(), host, &machine, keys, prompt)?;
                Ok(())
            })?;
            let code = match outcome {
                Outcome::Declined => return Ok(()),
                Outcome::Unchanged(commit) | Outcome::Pushed(commit) => commit,
            };
            trust::github::add(&fleet::admin()?, &uploads)?;
            println!("On {host}, type this code: {}", checked::short(&code));
            Ok(())
        }
        (Command::Retire, [host]) => {
            system::host::check(host)?;
            let outcome = edit_fleet(&format!("Retire {host}"), |fleet, _| {
                trust::retire_host(fleet, host)
            })?;
            match outcome {
                Outcome::Declined => Ok(()),
                Outcome::Unchanged(_) | Outcome::Pushed(_) => {
                    trust::github::remove(&fleet::admin()?, host)
                }
            }
        }
        (Command::Secret, [service, name]) => {
            let dir = std::path::Path::new(fleet::DIR);
            let machine = fleet::Here::load()?.machine(&fleet::machines::Machines::load(dir)?)?;
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
) -> anyhow::Result<Outcome> {
    let key = trust::sign::require_machine_key()?;
    let mut prompt = Prompt::new();
    let Some(stage) = checked::stage(&mut prompt)? else {
        return Ok(Outcome::Declined);
    };
    let fleet = Fleet::load_at(&stage.dir)?;
    command(&fleet, &mut prompt)?;
    let who = checked::author(fleet.admin())?;
    stage.commit(
        fleet.admin(),
        &who,
        message,
        &key,
        &mut prompt,
        Advance::Merge,
    )
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
    let _lock = match interface {
        Some(interface) => {
            services::start::wait_for_address(interface)?;
            wait_for_lock()?
        }
        None => match lock()? {
            Some(lock) => lock,
            None => {
                println!("Another mothership command is running, and it starts the services");
                return Ok(());
            }
        },
    };
    services::start::command(interface)
}

fn lock() -> anyhow::Result<Option<std::fs::File>> {
    let file = lock_file()?;
    match flock(&file, libc::LOCK_EX | libc::LOCK_NB) {
        Ok(()) => Ok(Some(file)),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
        Err(e) => Err(e).context("locking mothership"),
    }
}

fn wait_for_lock() -> anyhow::Result<std::fs::File> {
    let file = lock_file()?;
    flock(&file, libc::LOCK_EX).context("locking mothership")?;
    Ok(file)
}

fn lock_file() -> anyhow::Result<std::fs::File> {
    std::fs::create_dir_all(LOCK_DIR)?;
    let path = std::path::Path::new(LOCK_DIR).join("lock");
    std::fs::File::create(&path).with_context(|| format!("opening {}", path.display()))
}

fn flock(file: &std::fs::File, operation: libc::c_int) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    match unsafe { libc::flock(file.as_raw_fd(), operation) } {
        0 => Ok(()),
        _ => Err(std::io::Error::last_os_error()),
    }
}

const LOCK_DIR: &str = "/var/lib/mothership";
