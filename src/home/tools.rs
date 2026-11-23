use crate::{
    fleet::{Installer, Manager, Tool},
    system::user::User,
    util::cmd::{self, As},
};
use anyhow::Result;

pub fn install(user: &User, manager: Manager, tool: &Tool) -> Result<()> {
    let spec = spec(manager, tool);
    let spec: Vec<&str> = spec.iter().map(String::as_str).collect();
    let who = As::User(user);
    match manager {
        Manager::Cargo => cmd::run(who, "cargo", &[&["install"], &spec[..]].concat()),
        Manager::Uv => cmd::run(who, "uv", &[&["tool", "install"], &spec[..]].concat()),
        Manager::Bun => cmd::run(who, &bun(user), &[&["add", "--global"], &spec[..]].concat()),
    }
}

pub fn uninstall(user: &User, manager: Manager, name: &str) -> Result<()> {
    let who = As::User(user);
    match manager {
        Manager::Cargo => cmd::run(who, "cargo", &["uninstall", name]),
        Manager::Uv => cmd::run(who, "uv", &["tool", "uninstall", name]),
        Manager::Bun => cmd::run(who, &bun(user), &["remove", "--global", name]),
    }
}

pub fn run_installer(user: &User, installer: &Installer) -> Result<()> {
    cmd::run(As::User(user), "sh", &["-c", &installer.run])
}

const BUN: &str = ".bun/bin/bun";
pub const BUN_GLOBALS: &str = ".bun/install/global/package.json";

pub fn has(user: &User, manager: Manager) -> bool {
    match manager {
        Manager::Bun => user.home.join(BUN).exists(),
        _ => cmd::found(As::User(user), manager.name()),
    }
}

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
        (Manager::Bun, None) => vec![format!("git+{}", tool.git)],
        (Manager::Bun, Some(branch)) => vec![format!("git+{}#{branch}", tool.git)],
    }
}

fn bun(user: &User) -> String {
    user.home.join(BUN).to_string_lossy().into_owned()
}

#[cfg(test)]
#[path = "../../tests/home/tools.rs"]
mod tests;
