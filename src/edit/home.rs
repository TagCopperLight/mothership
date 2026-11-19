use super::{FILE_MODE, append_line};
use crate::{
    chezmoi::{self, ENCRYPTED, TEMPLATE},
    file,
    fleet::{Fleet, Member},
    homes, pref,
    preview::Seen,
    prompt::Prompt,
    secrets::{AGE, Storage},
    snapshot::Snapshot,
    style, trust, walk,
};
use anyhow::{Context, Result, bail};
use std::{
    ffi::OsStr,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub fn track(
    fleet: &Fleet,
    member: &Member,
    path: &Path,
    storage: Storage,
    prompt: &mut Prompt,
    seen: &Seen,
) -> Result<bool> {
    let root = member.chezmoi_root(fleet);
    let snapshot = taken(member, path, seen)?;
    let shown = snapshot.path(member.user(), path)?;
    if shown.is_dir() {
        if storage == Storage::Clear {
            refuse_private(&shown)?;
        }
        let count = walk::files(&shown)?.len();
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
    seen: &Seen,
) -> Result<bool> {
    let root = member.chezmoi_root(fleet);
    let source = root.source_path(path)?;
    let name = source
        .file_name()
        .context("the source has no name")?
        .to_string_lossy();
    if name.ends_with(TEMPLATE) {
        bail!("{} is a template, edit it by hand", source.display());
    }
    let clear = !name.starts_with(ENCRYPTED);
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

fn taken(member: &Member, path: &Path, seen: &Seen) -> Result<Snapshot> {
    let snapshot = Snapshot::take(member.user(), &[path])?;
    seen.check(snapshot.digest())?;
    Ok(snapshot)
}

fn files_in(shown: &Path) -> Result<Vec<(PathBuf, fs::Metadata)>> {
    let meta = fs::symlink_metadata(shown)?;
    if !meta.is_dir() {
        return Ok(vec![(shown.to_path_buf(), meta)]);
    }
    Ok(walk::files_with_metadata(shown)?
        .into_iter()
        .map(|(rel, meta)| (shown.join(rel), meta))
        .collect())
}

fn refuse_private(shown: &Path) -> Result<()> {
    let names: Vec<String> = files_in(shown)?
        .into_iter()
        .filter(|(_, meta)| walk::private(meta))
        .map(|(file, _)| {
            file.strip_prefix(shown)
                .map(|rel| rel.display().to_string())
        })
        .collect::<Result<_, _>>()?;
    if !names.is_empty() {
        bail!(
            "the folder holds files only their owner can read ({}): add them encrypted, one by one",
            names.join(", ")
        );
    }
    Ok(())
}

fn keep_owner_only(shown: &Path) -> Result<()> {
    for (file, meta) in files_in(shown)? {
        if meta.is_file() {
            let mode = meta.permissions().mode() & 0o700;
            fs::set_permissions(&file, fs::Permissions::from_mode(mode))?;
        }
    }
    Ok(())
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
    if !fs::symlink_metadata(source)?.is_file() {
        bail!(
            "chezmoi left {} as a link, as the file matches the fleet again",
            source.display()
        );
    }
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
    let args = [OsStr::new("add"), path.as_os_str()];
    if storage == Storage::Clear {
        root.output(snapshot, &args)?;
        return Ok(());
    }
    if !member.open {
        bail!(
            "{}'s key isn't in {}: `mothership trust` it first",
            member.name,
            member.recipients().display()
        );
    }
    let user = member.user();
    let shown = snapshot.path(user, path)?;
    user.with_rights(|| keep_owner_only(path))?;
    keep_owner_only(&shown)?;
    let args = [OsStr::new("add"), OsStr::new("--encrypt"), path.as_os_str()];
    root.run(&own_list(fleet, member)?, snapshot, &args)
}

fn set_pref(source: &Path, key: &str, value: &str) -> Result<()> {
    let name = source.to_string_lossy();
    if name.ends_with(AGE) || name.ends_with(TEMPLATE) {
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
    file::write(source, (out.join("\n") + "\n").as_bytes(), FILE_MODE)
}

#[cfg(test)]
#[path = "../../tests/edit_home.rs"]
mod tests;
