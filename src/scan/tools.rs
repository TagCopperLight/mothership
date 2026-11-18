use super::{Change, Item, Kind, join};
use crate::{
    cmd::{self, As},
    edit::{Add, Key},
    fleet::{GitTool, Manager, Member, Tool, Tools},
    revert::Revert,
    user::User,
};
use anyhow::{Context, Result};
use std::{collections::BTreeMap, fs, io::ErrorKind, path::Path, thread};

pub fn scan(member: &Member) -> Result<Vec<Item>> {
    let user = member.user();
    let tools = &member.config.tools;
    let mut items: Vec<Item> = thread::scope(|s| {
        let handles: Vec<_> = Manager::ALL
            .into_iter()
            .filter(|&manager| uses(user, tools, manager))
            .map(|manager| s.spawn(move || Ok(compare(tools, manager, &installed(user, manager)?))))
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
                .reverting(Revert::RunInstaller(installer.name.clone()))
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

fn uses(user: &User, tools: &Tools, manager: Manager) -> bool {
    !tools.of(manager).is_empty() || crate::tools::has(user, manager)
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
                .reverting(Revert::UninstallTool {
                    manager,
                    name: name.clone(),
                }),
            Some(tool) if *source == Source::Declarable(tool.clone()) => continue,
            Some(tool) => Item::new(Kind::Tool, Change::Edited, subject(name), "other source")
                .noting(format!("from {machine}, the fleet says {}", from(tool)))
                .reverting(Revert::ReinstallTool {
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
                .reverting(Revert::InstallTool {
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
        Manager::Uv => uv_tools(Path::new(cmd::stdout(who, "uv", &["tool", "dir"])?.trim())),
        Manager::Bun => bun_globals(&user.home.join(".bun/install/global/package.json")),
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

/// `query` is the one key that names a branch: cargo writes `?branch=`, and uv `?rev=` for the
/// `@branch` apply gives it. Any other query, such as a tag, can't be declared.
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
    let entries = match fs::read_dir(dir) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(tools),
        entries => entries.with_context(|| format!("reading {}", dir.display()))?,
    };
    for entry in entries {
        let receipt = entry?.path().join("uv-receipt.toml");
        if !receipt.exists() {
            continue;
        }
        let text = fs::read_to_string(&receipt)?;
        let (name, source) =
            uv_receipt(&text).with_context(|| format!("reading {}", receipt.display()))?;
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
    let text = match fs::read_to_string(path) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(BTreeMap::new()),
        text => text.with_context(|| format!("reading {}", path.display()))?,
    };
    let manifest: serde_json::Value = serde_json::from_str(&text)?;
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
#[path = "../../tests/tools_scan.rs"]
mod tests;
