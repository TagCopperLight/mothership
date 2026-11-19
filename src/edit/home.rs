use super::append_line;
use crate::{
    chezmoi, file,
    fleet::{Fleet, Member},
    homes, pref,
    prompt::Prompt,
    secrets::Storage,
    snapshot::Snapshot,
    style, trust, walk,
};
use anyhow::{Context, Result, bail};
use std::{ffi::OsStr, fs, path::Path};

pub fn track(
    fleet: &Fleet,
    member: &Member,
    path: &Path,
    storage: Storage,
    prompt: &mut Prompt,
    seen: Option<&str>,
) -> Result<bool> {
    let root = member.chezmoi_root(fleet);
    let snapshot = taken(member, path, seen)?;
    if snapshot.path(member.user(), path)?.is_dir() {
        let count = walk::files(&snapshot.path(member.user(), path)?)?.len();
        let files = style::count(count, "file");
        if !prompt.confirm(&format!("  That adds {files}. Go on"))? {
            return Ok(false);
        }
    }
    chezmoi_add(fleet, member, &root, &snapshot, path, storage)?;
    Ok(true)
}

pub fn update(
    fleet: &Fleet,
    member: &Member,
    path: &Path,
    storage: Storage,
    prompt: &mut Prompt,
    seen: Option<&str>,
) -> Result<bool> {
    let root = member.chezmoi_root(fleet);
    let source = root.source_path(path)?;
    if source.extension().is_some_and(|e| e == "tmpl") {
        bail!("{} is a template, edit it by hand", source.display());
    }
    let clear = !source
        .file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with("encrypted_"));
    let link = fs::symlink_metadata(&source)?.is_symlink();
    if link && storage == Storage::Encrypted && clear {
        bail!(
            "{} is shared: encrypt it in {} by hand",
            source.display(),
            homes::target(&fleet.dir, &source)?.display()
        );
    }
    if link && !confirm_shared(fleet, member, &source, prompt)? {
        return Ok(false);
    }
    let snapshot = taken(member, path, seen)?;
    if link {
        update_shared(fleet, &root, &snapshot, &source, path)?;
    } else if storage == Storage::Clear {
        let args = [OsStr::new("re-add"), path.as_os_str()];
        root.run(&own_list(fleet, member)?, &snapshot, &args)?;
    } else {
        chezmoi_add(fleet, member, &root, &snapshot, path, Storage::Encrypted)?;
        if clear {
            println!("  git history keeps the version in clear, so change it.");
        }
    }
    Ok(true)
}

pub fn forget(fleet: &Fleet, member: &Member, path: &Path) -> Result<()> {
    let root = member.chezmoi_root(fleet);
    let empty = Snapshot::take(member.user(), &[])?;
    let args = [
        OsStr::new("forget"),
        OsStr::new("--force"),
        path.as_os_str(),
    ];
    root.run(&own_list(fleet, member)?, &empty, &args)
}

pub fn pref(
    fleet: &Fleet,
    member: &Member,
    user_js: &Path,
    key: &str,
    value: &str,
    prompt: &mut Prompt,
) -> Result<bool> {
    let source = member.chezmoi_root(fleet).source_path(user_js)?;
    let file = match fs::symlink_metadata(&source)?.is_symlink() {
        true => {
            if !confirm_shared(fleet, member, &source, prompt)? {
                return Ok(false);
            }
            homes::target(&fleet.dir, &source)?
        }
        false => source,
    };
    set_pref(&file, key, value)?;
    Ok(true)
}

fn taken(member: &Member, path: &Path, seen: Option<&str>) -> Result<Snapshot> {
    let snapshot = Snapshot::take(member.user(), &[path])?;
    if seen.is_some_and(|seen| seen != snapshot.digest()) {
        bail!("it changed since it was shown: pass, then review it again");
    }
    Ok(snapshot)
}

fn confirm_shared(
    fleet: &Fleet,
    member: &Member,
    source: &Path,
    prompt: &mut Prompt,
) -> Result<bool> {
    let shared = homes::target(&fleet.dir, source)?;
    let others: Vec<String> = homes::users(&fleet.dir, source)?
        .into_iter()
        .filter(|user| *user != member.name)
        .collect();
    let shown = shared.strip_prefix(&fleet.dir).unwrap_or(&shared).display();
    println!(
        "  {shown} is shared: {}'s version reaches {} too.",
        member.name,
        others.join(" and ")
    );
    prompt.confirm("  Go on")
}

fn own_list(fleet: &Fleet, member: &Member) -> Result<String> {
    trust::home_list(&fleet.dir, &homes::source(&fleet.dir, &member.name))
}

fn update_shared(
    fleet: &Fleet,
    root: &chezmoi::Root,
    snapshot: &Snapshot,
    source: &Path,
    path: &Path,
) -> Result<()> {
    let shared = homes::target(&fleet.dir, source)?;
    let link = fs::read_link(source)?;
    let recipients = trust::home_list(&fleet.dir, source)?;
    root.run(
        &recipients,
        snapshot,
        &[OsStr::new("re-add"), path.as_os_str()],
    )?;
    fs::rename(source, &shared)?;
    file::link(source, &link)
}

fn chezmoi_add(
    fleet: &Fleet,
    member: &Member,
    root: &chezmoi::Root,
    snapshot: &Snapshot,
    path: &Path,
    storage: Storage,
) -> Result<()> {
    let path = path.as_os_str();
    let recipients = own_list(fleet, member)?;
    if storage == Storage::Clear {
        return root.run(&recipients, snapshot, &[OsStr::new("add"), path]);
    }
    if !member.open {
        bail!(
            "{}'s key isn't in {}: `mothership trust` it first",
            member.name,
            member.recipients().display()
        );
    }
    root.run(
        &recipients,
        snapshot,
        &[OsStr::new("add"), OsStr::new("--encrypt"), path],
    )
}

fn set_pref(source: &Path, key: &str, value: &str) -> Result<()> {
    if source
        .extension()
        .is_some_and(|e| e == "age" || e == "tmpl")
    {
        bail!(
            "{} is encrypted or a template: set the pref in it by hand",
            source.display()
        );
    }
    let line = pref::line(key, value);
    let text = fs::read_to_string(source)?;
    let sets_it = |l: &str| pref::parse_line(l).is_some_and(|(k, _)| k == key);
    if !text.lines().any(sets_it) {
        return append_line(source, &line);
    }
    let out: Vec<&str> = text
        .lines()
        .map(|l| if sets_it(l) { line.as_str() } else { l })
        .collect();
    fs::write(source, out.join("\n") + "\n")
        .with_context(|| format!("writing {}", source.display()))
}

#[cfg(test)]
#[path = "../../tests/edit_home.rs"]
mod tests;
