use crate::{
    cmd,
    fleet::{Installer, Manager, Tool},
    user,
};
use anyhow::Result;

/// Each runs as the user, since the tools live in their home.
pub fn install(manager: Manager, tool: &Tool) -> Result<()> {
    let spec = spec(manager, tool);
    let spec: Vec<&str> = spec.iter().map(String::as_str).collect();
    match manager {
        Manager::Cargo => cmd::run("cargo", &[&["install"], &spec[..]].concat()),
        Manager::Uv => cmd::run("uv", &[&["tool", "install"], &spec[..]].concat()),
        Manager::Bun => cmd::run(&bun(), &[&["add", "--global"], &spec[..]].concat()),
    }
}

pub fn uninstall(manager: Manager, name: &str) -> Result<()> {
    match manager {
        Manager::Cargo => cmd::run("cargo", &["uninstall", name]),
        Manager::Uv => cmd::run("uv", &["tool", "uninstall", name]),
        Manager::Bun => cmd::run(&bun(), &["remove", "--global", name]),
    }
}

pub fn run_installer(installer: &Installer) -> Result<()> {
    cmd::run("sh", &["-c", &installer.run])
}

/// What follows the manager's install command.
fn spec(manager: Manager, tool: &Tool) -> Vec<String> {
    let Tool::Git(tool) = tool else {
        return vec![tool.name().to_owned()];
    };
    let name = &tool.name;
    match (manager, &tool.branch) {
        (Manager::Cargo, None) => vec!["--git".into(), tool.git.clone(), name.clone()],
        (Manager::Cargo, Some(branch)) => vec![
            "--git".into(),
            tool.git.clone(),
            "--branch".into(),
            branch.clone(),
            name.clone(),
        ],
        (Manager::Uv, None) => vec![format!("{name} @ git+{}", tool.git)],
        (Manager::Uv, Some(branch)) => vec![format!("{name} @ git+{}@{branch}", tool.git)],
        // bun refuses a name before a git source, and takes the one in the package.
        (Manager::Bun, None) => vec![format!("git+{}", tool.git)],
        (Manager::Bun, Some(branch)) => vec![format!("git+{}#{branch}", tool.git)],
    }
}

/// bun's installer puts it in the home, out of sudo's PATH.
fn bun() -> String {
    user::home()
        .join(".bun/bin/bun")
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
#[path = "../tests/tools.rs"]
mod tests;
