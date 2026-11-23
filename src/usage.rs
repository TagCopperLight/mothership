use crate::util::file;
use anyhow::Result;
use std::path::Path;

const COMMANDS: [(&str, &str, &str); 9] = [
    ("scan", "", "show what differs from the fleet"),
    ("review", "", "go through this machine's changes"),
    ("apply", "", "bring in the fleet's changes"),
    (
        "commit",
        " <message>",
        "sign and push your edits in the fleet's folder",
    ),
    ("recovery", "", "name or replace the fleet's recovery key"),
    (
        "trust",
        " <host> <type> <key>...",
        "let a new machine's keys in",
    ),
    ("retire", " <host>", "remove a machine's keys"),
    (
        "secret",
        " <service> <name>",
        "encrypt a value into a service's secrets",
    ),
    (
        "install",
        " [options]",
        "set up this machine from the fleet",
    ),
];

pub const INSTALL_FLAGS: [(&str, &str); 9] = [
    ("fleet", "<url>"),
    ("type", "<type>"),
    ("option", "<option>"),
    ("hostname", "<name>"),
    ("admin", "<user>"),
    ("remove-unlisted", "yes|no"),
    ("timezone", "<zone>"),
    ("disk", "<disk>"),
    ("layout", "wipe|beside"),
];

const FISH: &str = "/usr/share/fish/vendor_completions.d/mothership.fish";

pub fn text() -> String {
    let width = COMMANDS
        .iter()
        .map(|(name, args, _)| name.len() + args.len())
        .max()
        .unwrap_or(0);
    let mut text = String::from("usage: sudo mothership <command>\n");
    for (name, args, about) in COMMANDS {
        text.push_str(&format!("  {:width$}  {about}\n", format!("{name}{args}")));
    }
    text.push_str("`mothership install --help` lists install's options.");
    text
}

pub fn of(name: &str) -> String {
    let (_, args, _) = COMMANDS
        .iter()
        .find(|(command, ..)| *command == name)
        .expect("only a known command has the wrong arguments");
    format!("sudo mothership {name}{args}")
}

pub fn install() -> String {
    let mut text = String::from("usage: sudo mothership install [options]\n");
    text.push_str("Each option answers one of install's questions:\n");
    for (flag, value) in INSTALL_FLAGS {
        text.push_str(&format!("  --{flag} {value}\n"));
    }
    text.push_str(
        "--option repeats, and --timezone, --disk and --layout work only from the Arch ISO.",
    );
    text
}

pub fn fish() -> String {
    let mut lines = vec!["complete -c mothership -f".to_owned()];
    for (name, _, about) in COMMANDS {
        lines.push(format!(
            "complete -c mothership -n __fish_use_subcommand -a {name} -d '{about}'"
        ));
    }
    for (flag, value) in INSTALL_FLAGS {
        let values = match value.contains('|') {
            true => format!(" -a '{}'", value.replace('|', " ")),
            false => String::new(),
        };
        lines.push(format!(
            "complete -c mothership -n '__fish_seen_subcommand_from install' -l {flag} -x{values}"
        ));
    }
    lines.join("\n") + "\n"
}

pub fn keep_completion() -> Result<()> {
    let path = Path::new(FISH);
    if !Path::new("/usr/share/fish").is_dir()
        || file::read_if_any(path)?.as_deref() == Some(fish().as_str())
    {
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().expect("the completion has a folder"))?;
    file::write(path, fish().as_bytes(), 0o644)
}

#[cfg(test)]
#[path = "../tests/usage.rs"]
mod tests;
