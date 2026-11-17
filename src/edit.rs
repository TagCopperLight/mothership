use crate::{cmd, fleet::Fleet, prompt::Prompt, walk};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    io::ErrorKind,
    os::unix,
    path::{Path, PathBuf},
};
use toml_edit::{Array, DocumentMut, Value, value};

/// A string array in system.toml.
#[derive(Clone, Copy)]
pub enum Key {
    Services,
    UserServices,
    Groups,
    Cargo,
    Uv,
    Bun,
}

impl Key {
    fn path(self) -> &'static [&'static str] {
        match self {
            Key::Services => &["services"],
            Key::UserServices => &["user_services"],
            Key::Groups => &["groups"],
            Key::Cargo => &["tools", "cargo"],
            Key::Uv => &["tools", "uv"],
            Key::Bun => &["tools", "bun"],
        }
    }

    pub fn name(self) -> &'static str {
        self.path().last().expect("paths are not empty")
    }
}

/// How to make the fleet accept what this machine has.
pub enum Add {
    ListPackage(String),
    UnlistPackage(String),
    Declare {
        key: Key,
        name: String,
    },
    Undeclare {
        key: Key,
        name: String,
    },
    UndeclareInstaller(String),
    Shell(String),
    Track(PathBuf),
    Update(PathBuf),
    CopyEtc(PathBuf),
    RemoveEtc(PathBuf),
    Pref {
        user_js: PathBuf,
        key: String,
        value: String,
    },
}

impl Add {
    pub fn describe(&self) -> String {
        match self {
            Add::ListPackage(_) => "list it in one of the type's package lists".into(),
            Add::UnlistPackage(_) => "remove it from the package lists".into(),
            Add::Declare { key, .. } => {
                format!("add it to `{}` in system.toml", key.path().join("."))
            }
            Add::Undeclare { key, .. } => {
                format!("remove it from `{}` in system.toml", key.path().join("."))
            }
            Add::UndeclareInstaller(_) => "remove its installer from system.toml".into(),
            Add::Shell(shell) => format!("make {shell} the type's shell"),
            Add::Track(_) => "add it to chezmoi".into(),
            Add::Update(_) => "copy this machine's version into chezmoi".into(),
            Add::CopyEtc(_) => "copy it into the type's etc/".into(),
            Add::RemoveEtc(_) => "remove it from the type's etc/".into(),
            Add::Pref { .. } => "write this machine's value into user.js".into(),
        }
    }

    /// Returns false when I cancel along the way.
    pub fn apply(&self, fleet: &Fleet, prompt: &mut Prompt) -> Result<bool> {
        match self {
            Add::ListPackage(name) => {
                let Some(list) = prompt.choose("  Which list", &fleet.system.packages)? else {
                    return Ok(false);
                };
                append_line(&fleet.package_path(list), name)?;
            }
            Add::UnlistPackage(name) => unlist_package(fleet, name)?,
            Add::Declare { key, name } => edit_system(fleet, |doc| {
                let array = array(doc, *key)?;
                let prefix = array
                    .iter()
                    .last()
                    .map(|last| last.decor().prefix().cloned());
                array.push(name.as_str());
                if let Some(Some(prefix)) = prefix {
                    let pushed = array.iter_mut().last().expect("just pushed");
                    pushed.decor_mut().set_prefix(prefix);
                }
                Ok(())
            })?,
            Add::Undeclare { key, name } => edit_system(fleet, |doc| {
                let array = array(doc, *key)?;
                let before = array.len();
                array.retain(|v| !names(v, name));
                if array.len() == before {
                    bail!("{name} is not in `{}`", key.path().join("."));
                }
                Ok(())
            })?,
            Add::UndeclareInstaller(name) => edit_system(fleet, |doc| {
                let installers = doc["installers"]
                    .as_array_of_tables_mut()
                    .context("system.toml has no [[installers]]")?;
                installers.retain(|t| t.get("name").and_then(|n| n.as_str()) != Some(name));
                Ok(())
            })?,
            Add::Shell(shell) => edit_system(fleet, |doc| {
                doc["shell"] = value(shell.as_str());
                Ok(())
            })?,
            Add::Track(path) => {
                if path.is_dir() {
                    let count = walk::files(path)?.len();
                    if !prompt.confirm(&format!("  That adds {count} files. Go on"))? {
                        return Ok(false);
                    }
                }
                cmd::run("chezmoi", &["add", &path.to_string_lossy()])?;
            }
            Add::Update(path) => {
                let source = source_path(path)?;
                if source.extension().is_some_and(|e| e == "tmpl") {
                    bail!("{} is a template, edit it by hand", source.display());
                }
                cmd::run("chezmoi", &["re-add", &path.to_string_lossy()])?;
            }
            Add::CopyEtc(path) => copy_etc(&etc_source(fleet, path), path)?,
            Add::RemoveEtc(path) => remove_etc(fleet, &etc_source(fleet, path))?,
            Add::Pref {
                user_js,
                key,
                value,
            } => set_pref(&source_path(user_js)?, key, value)?,
        }
        Ok(true)
    }
}

fn append_line(path: &Path, line: &str) -> Result<()> {
    let mut text =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(line);
    text.push('\n');
    fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

pub fn ignore(fleet: &Fleet, line: &str) -> Result<()> {
    append_line(&fleet.ignore_path(), line)
}

/// Takes the name off every list of the type, keeping the comments lined up.
fn unlist_package(fleet: &Fleet, name: &str) -> Result<()> {
    let mut found = false;
    for list in fleet
        .system
        .packages
        .iter()
        .chain(&fleet.system.built_packages)
    {
        let path = fleet.package_path(list);
        let text = fs::read_to_string(&path)?;
        let mut out = String::new();
        for line in text.lines() {
            let (code, comment) = line.split_at(line.find('#').unwrap_or(line.len()));
            let names: Vec<&str> = code.split_whitespace().collect();
            if !names.contains(&name) {
                out.push_str(line);
            } else if names.len() > 1 {
                found = true;
                let kept: Vec<&str> = names.into_iter().filter(|n| *n != name).collect();
                let code = format!("{:width$}", kept.join(" "), width = code.len());
                out.push_str(if comment.is_empty() {
                    code.trim_end()
                } else {
                    &code
                });
                out.push_str(comment);
            } else {
                found = true;
                continue;
            }
            out.push('\n');
        }
        fs::write(&path, out)?;
    }
    if !found {
        bail!("{name} is in none of the type's lists");
    }
    Ok(())
}

fn edit_system(fleet: &Fleet, change: impl FnOnce(&mut DocumentMut) -> Result<()>) -> Result<()> {
    let path = fleet.type_dir.join("system.toml");
    let mut doc: DocumentMut = fs::read_to_string(&path)?
        .parse()
        .with_context(|| format!("parsing {}", path.display()))?;
    change(&mut doc)?;
    fs::write(&path, doc.to_string()).with_context(|| format!("writing {}", path.display()))
}

fn array(doc: &mut DocumentMut, key: Key) -> Result<&mut Array> {
    let (last, parents) = key.path().split_last().expect("paths are not empty");
    let mut table = doc.as_table_mut();
    for parent in parents {
        table = table
            .entry(parent)
            .or_insert(toml_edit::table())
            .as_table_mut()
            .with_context(|| format!("`{parent}` in system.toml is not a table"))?;
    }
    table
        .entry(last)
        .or_insert(value(Array::new()))
        .as_array_mut()
        .with_context(|| format!("`{}` in system.toml is not an array", key.path().join(".")))
}

/// A tool is either its name or a table with its name and source.
fn names(entry: &Value, name: &str) -> bool {
    entry.as_str() == Some(name)
        || entry
            .as_inline_table()
            .and_then(|t| t.get("name"))
            .and_then(Value::as_str)
            == Some(name)
}

fn source_path(target: &Path) -> Result<PathBuf> {
    Ok(PathBuf::from(
        cmd::stdout("chezmoi", &["source-path", &target.to_string_lossy()])?.trim(),
    ))
}

fn etc_source(fleet: &Fleet, path: &Path) -> PathBuf {
    let rel = path
        .strip_prefix("/etc")
        .expect("the /etc scan reports paths under /etc");
    fleet.type_dir.join("etc").join(rel)
}

fn copy_etc(dest: &Path, path: &Path) -> Result<()> {
    fs::create_dir_all(dest.parent().expect("under etc/"))?;
    let meta = fs::symlink_metadata(path)?;
    if meta.is_symlink() {
        match fs::remove_file(dest) {
            Err(e) if e.kind() != ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        unix::fs::symlink(fs::read_link(path)?, dest)?;
    } else if meta.is_dir() {
        bail!(
            "{} is a folder: copy the files you want one by one",
            path.display()
        );
    } else {
        let bytes = match fs::read(path) {
            Err(e) if e.kind() == ErrorKind::PermissionDenied => {
                bail!("{} needs root to read", path.display())
            }
            read => read?,
        };
        fs::write(dest, bytes)?;
    }
    Ok(())
}

/// Removes the file, then the folders it leaves empty.
fn remove_etc(fleet: &Fleet, dest: &Path) -> Result<()> {
    fs::remove_file(dest).with_context(|| format!("removing {}", dest.display()))?;
    let root = fleet.type_dir.join("etc");
    for dir in dest.ancestors().skip(1).take_while(|dir| *dir != root) {
        if fs::read_dir(dir)?.next().is_some() {
            break;
        }
        fs::remove_dir(dir)?;
    }
    Ok(())
}

fn set_pref(source: &Path, key: &str, value: &str) -> Result<()> {
    let start = format!("user_pref(\"{key}\",");
    let line = format!("user_pref(\"{key}\", {value});");
    let text = fs::read_to_string(source)?;
    if text.lines().any(|l| l.trim_start().starts_with(&start)) {
        let out: Vec<&str> = text
            .lines()
            .map(|l| {
                if l.trim_start().starts_with(&start) {
                    line.as_str()
                } else {
                    l
                }
            })
            .collect();
        fs::write(source, out.join("\n") + "\n")?;
        Ok(())
    } else {
        append_line(source, &line)
    }
}
