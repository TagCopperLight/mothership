use super::join;
use crate::sync::item::{Change, Item};
use crate::sync::kind::Kind;
use crate::{
    fleet::edit::{Add, Key},
    fleet::{GitTool, Manager, Member, Tool, Tools},
    sync::revert,
    system::user::User,
    util::cmd::{self, As},
    util::file,
    util::walk,
};
use anyhow::{Context, Result};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    thread,
};
use strum::IntoEnumIterator;

pub fn scan(member: &Member) -> Result<Vec<Item>> {
    let user = member.user();
    let tools = &member.config.tools;
    let mut items: Vec<Item> = thread::scope(|s| {
        let handles: Vec<_> = Manager::iter()
            .filter_map(|manager| {
                let has = crate::home::tools::has(user, manager);
                (has || !tools.of(manager).is_empty()).then(|| {
                    s.spawn(move || {
                        let installed = match has {
                            true => installed(user, manager)?,
                            false => BTreeMap::new(),
                        };
                        Ok(compare(tools, manager, &installed))
                    })
                })
            })
            .collect();
        handles.into_iter().map(join).collect::<Result<Vec<_>>>()
    })?
    .into_iter()
    .flatten()
    .collect();
    items.extend(
        member
            .config
            .installers
            .iter()
            .filter(|installer| !user.expand(&installer.creates).exists())
            .map(|installer| {
                Item::new(
                    Kind::Tool,
                    Change::Removed,
                    installer.name.as_str(),
                    "not installed",
                )
                .noting(format!("{} is missing", installer.creates))
                .with(Add::UndeclareInstaller(installer.name.clone()))
                .reverting(revert::User::RunInstaller(installer.name.clone()))
            }),
    );
    Ok(items)
}

#[derive(Debug, PartialEq)]
enum Source {
    Declarable(Tool),
    Other(String),
}

fn from(tool: &Tool) -> String {
    match tool {
        Tool::Name(_) => "the registry".into(),
        Tool::Git(GitTool {
            git, branch: None, ..
        }) => git.clone(),
        Tool::Git(GitTool {
            git,
            branch: Some(branch),
            ..
        }) => format!("{git}, branch {branch}"),
    }
}

fn compare(tools: &Tools, manager: Manager, installed: &BTreeMap<String, Source>) -> Vec<Item> {
    let declared = tools.of(manager);
    let subject = |name: &str| format!("{} {name}", manager.name());
    let mut items = Vec::new();
    for (name, source) in installed {
        let (machine, add) = match source {
            Source::Declarable(tool) => (
                from(tool),
                Some(Add::DeclareTool {
                    manager,
                    tool: tool.clone(),
                }),
            ),
            Source::Other(text) => (text.clone(), None),
        };
        let item = match tools.find(manager, name) {
            None => Item::new(Kind::Tool, Change::Added, subject(name), "installed")
                .noting(format!("from {machine}"))
                .reverting(revert::User::UninstallTool {
                    manager,
                    name: name.clone(),
                }),
            Some(tool) if *source == Source::Declarable(tool.clone()) => continue,
            Some(tool) => Item::new(Kind::Tool, Change::Edited, subject(name), "other source")
                .noting(format!("from {machine}, the fleet says {}", from(tool)))
                .reverting(revert::User::ReinstallTool {
                    manager,
                    tool: tool.clone(),
                }),
        };
        items.push(match add {
            Some(add) => item.with(add),
            None => item.because("the fleet can't describe this source"),
        });
    }
    items.extend(
        declared
            .iter()
            .filter(|tool| !installed.contains_key(tool.name()))
            .map(|tool| {
                Item::new(
                    Kind::Tool,
                    Change::Removed,
                    subject(tool.name()),
                    "uninstalled",
                )
                .with(Add::Undeclare {
                    key: Key::Tools(manager),
                    name: tool.name().to_owned(),
                })
                .reverting(revert::User::InstallTool {
                    manager,
                    tool: tool.clone(),
                })
            }),
    );
    items
}

fn installed(user: &User, manager: Manager) -> Result<BTreeMap<String, Source>> {
    let who = As::User(user);
    match manager {
        Manager::Cargo => Ok(cargo_list(&cmd::stdout(
            who,
            "cargo",
            &["install", "--list"],
        )?)),
        Manager::Uv => {
            let dir = PathBuf::from(cmd::stdout(who, "uv", &["tool", "dir"])?.trim());
            user.with_rights(|| uv_tools(&dir))
        }
        Manager::Bun => {
            let globals = user.home.join(crate::home::tools::BUN_GLOBALS);
            user.with_rights(|| bun_globals(&globals))
        }
    }
}

fn cargo_list(text: &str) -> BTreeMap<String, Source> {
    text.lines()
        .filter(|line| !line.starts_with(char::is_whitespace))
        .filter_map(|line| {
            let line = line.strip_suffix(':')?;
            let name = line.split_whitespace().next()?.to_owned();
            let source = match line.split_once(" (") {
                None => Source::Declarable(Tool::Name(name.clone())),
                Some((_, origin)) => {
                    let origin = origin.strip_suffix(')').unwrap_or(origin);
                    match origin.split_once('#') {
                        Some((url, _commit)) => git_source(&name, url, "branch"),
                        None => Source::Other(origin.to_owned()),
                    }
                }
            };
            Some((name, source))
        })
        .collect()
}

fn git_source(name: &str, url: &str, query: &str) -> Source {
    let branch = match url.split_once('?') {
        None => None,
        Some((_, rest)) => match rest.strip_prefix(&format!("{query}=")) {
            Some(branch) if !branch.contains('&') => Some(branch.to_owned()),
            _ => return Source::Other(url.to_owned()),
        },
    };
    let git = url
        .split('?')
        .next()
        .expect("split has a first part")
        .to_owned();
    Source::Declarable(Tool::Git(GitTool {
        name: name.to_owned(),
        git,
        branch,
    }))
}

fn uv_tools(dir: &Path) -> Result<BTreeMap<String, Source>> {
    let mut tools = BTreeMap::new();
    for tool in walk::children(dir)? {
        if !tool.is_dir() {
            continue;
        }
        let receipt = tool.join("uv-receipt.toml");
        let Some(bytes) = file::read(&receipt)? else {
            continue;
        };
        let (name, source) = uv_receipt(&String::from_utf8_lossy(&bytes))
            .with_context(|| format!("reading {}", receipt.display()))?;
        tools.insert(name, source);
    }
    Ok(tools)
}

fn uv_receipt(text: &str) -> Result<(String, Source)> {
    let receipt: toml::Table = toml::from_str(text)?;
    let requirement = receipt
        .get("tool")
        .and_then(|tool| tool.get("requirements"))
        .and_then(|r| r.as_array())
        .and_then(|r| r.first())
        .and_then(|r| r.as_table())
        .context("no requirement")?;
    let name = requirement
        .get("name")
        .and_then(|n| n.as_str())
        .context("the requirement has no name")?
        .to_owned();
    let mut keys: Vec<&str> = requirement.keys().map(String::as_str).collect();
    keys.sort();
    let source = match (
        keys.as_slice(),
        requirement.get("git").and_then(|g| g.as_str()),
    ) {
        (["name"], _) => Source::Declarable(Tool::Name(name.clone())),
        (["git", "name"], Some(url)) => git_source(&name, url, "rev"),
        _ => Source::Other(
            requirement
                .iter()
                .filter(|(key, _)| *key != "name")
                .map(|(key, value)| format!("{key} {value}"))
                .collect::<Vec<_>>()
                .join(", "),
        ),
    };
    Ok((name, source))
}

fn bun_globals(path: &Path) -> Result<BTreeMap<String, Source>> {
    let Some(bytes) = file::read(path)? else {
        return Ok(BTreeMap::new());
    };
    let manifest: serde_json::Value =
        serde_json::from_slice(&bytes).with_context(|| format!("reading {}", path.display()))?;
    let Some(deps) = manifest["dependencies"].as_object() else {
        return Ok(BTreeMap::new());
    };
    deps.iter()
        .map(|(name, spec)| {
            let spec = spec.as_str().context("a dependency that is not a string")?;
            Ok((name.clone(), bun_source(name, spec)))
        })
        .collect()
}

fn bun_source(name: &str, spec: &str) -> Source {
    if let Some(url) = spec.strip_prefix("git+") {
        let (git, branch) = match url.split_once('#') {
            Some((git, branch)) => (git, Some(branch.to_owned())),
            None => (url, None),
        };
        return Source::Declarable(Tool::Git(GitTool {
            name: name.to_owned(),
            git: git.to_owned(),
            branch,
        }));
    }
    if spec.starts_with(['^', '~', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9']) {
        Source::Declarable(Tool::Name(name.to_owned()))
    } else {
        Source::Other(spec.to_owned())
    }
}

#[cfg(test)]
#[path = "../../../tests/sync/scan/tools.rs"]
mod tests;
