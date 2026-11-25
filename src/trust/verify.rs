use super::sign::{self, Reach, Signer};
use crate::{
    fleet::git::{Git, short},
    services,
    util::style,
};
use anyhow::{Context, Result, bail};
use ssh_key::PrivateKey;

const PLAIN: &str = "100644";

#[derive(Debug)]
pub struct Refused {
    pub commit: String,
    pub author: String,
    pub date: String,
    pub subject: String,
    pub why: String,
}

pub struct Checked {
    pub commit: String,
    pub refused: Option<Refused>,
    pub steps: Vec<Step>,
}

pub struct Step {
    pub parent: String,
    pub commit: String,
    pub reach: Reach,
}

pub fn check(git: &Git, from: &str, to: &str) -> Result<Checked> {
    follows(git, from, to)?;
    let list = git.out(&[
        "rev-list",
        "--first-parent",
        "--reverse",
        &format!("{from}..{to}"),
    ])?;
    let mut good = from.to_owned();
    let mut refused = None;
    let mut steps = Vec::new();
    for commit in list.lines() {
        match admit(git, &good, commit, refused.as_ref()) {
            Ok(signer) => {
                steps.push(Step {
                    parent: good,
                    commit: commit.to_owned(),
                    reach: signer.map_or(Reach::Everything, |signer| signer.reach),
                });
                good = commit.to_owned();
                refused = None;
            }
            Err(why) if refused.is_none() => refused = Some(refused_at(git, commit, &why)?),
            Err(_) => {}
        }
    }
    Ok(Checked {
        commit: good,
        refused,
        steps,
    })
}

pub fn follows(git: &Git, from: &str, to: &str) -> Result<()> {
    if !git.is_ancestor(from, to)? {
        bail!(
            "main doesn't follow {}, the last commit this machine checked: rebase it onto that \
             commit",
            short(from)
        );
    }
    Ok(())
}

fn refused_at(git: &Git, commit: &str, why: &anyhow::Error) -> Result<Refused> {
    let line = git.out(&["log", "-1", "--format=%an%x00%as%x00%s", commit])?;
    let [author, date, subject] = line.splitn(3, '\0').collect::<Vec<_>>()[..] else {
        bail!("git log printed {line}");
    };
    Ok(Refused {
        commit: commit.to_owned(),
        author: style::clean(author),
        date: style::clean(date),
        subject: style::clean(subject),
        why: format!("{why:#}"),
    })
}

fn admit(git: &Git, good: &str, commit: &str, refused: Option<&Refused>) -> Result<Option<Signer>> {
    let Some(refused) = refused else {
        return verify(git, good, commit);
    };
    if git.rev(&format!("{}^1", refused.commit))?.as_deref() != Some(good) {
        bail!(
            "it can't vouch for {}, which doesn't follow {} on main's line",
            short(&refused.commit),
            short(good)
        );
    }
    let signer = approve(git, good, commit)?;
    if let Some(signer) = &signer
        && signer.reach == Reach::Versions
    {
        bail!(
            "{} may change only versions, so it can't vouch for the commits before it",
            signer.name
        );
    }
    Ok(signer)
}

pub fn verify(git: &Git, parent: &str, commit: &str) -> Result<Option<Signer>> {
    let first = git.out(&["rev-parse", &format!("{commit}^1")])?;
    if first != parent {
        bail!("it doesn't follow {} on main's line", short(parent));
    }
    approve(git, parent, commit)
}

fn approve(git: &Git, base: &str, commit: &str) -> Result<Option<Signer>> {
    let after = git.machines(commit)?;
    sign::distinct(&after.signers()?, git.recovery(commit)?.as_ref())?;
    let before = git.machines(base)?;
    let mut trusted = before.signers()?;
    trusted.extend(git.recovery(base)?);
    if trusted.is_empty() {
        return Ok(None);
    }
    let signed = git.commit(commit)?;
    let Some(signer) = signed.signer(&trusted) else {
        bail!(match signed.signature {
            None => "nobody signed it",
            Some(_) => "a key the fleet doesn't list signed it",
        });
    };
    let changed = git.changed(base, commit)?;
    sign::allowed(signer, &changed, &before, &after)?;
    if signer.reach == Reach::Versions {
        for path in &changed {
            let text = path.to_string_lossy();
            let (Some(old), Some(new)) = (git.blob(base, &text)?, git.blob(commit, &text)?) else {
                bail!("{} can't add or remove {text}", signer.name);
            };
            for side in [base, commit] {
                if git.out(&["ls-tree", "--format=%(objectmode)", side, "--", &text])? != PLAIN {
                    bail!(
                        "{} can change only a plain file's text, not {text}'s kind",
                        signer.name
                    );
                }
            }
            services::versions_only(&old, &new)
                .with_context(|| format!("{} can't sign this change to {text}", signer.name))?;
        }
    }
    Ok(Some(signer.clone()))
}

pub fn signed_commit(
    git: &Git,
    tree: &str,
    parent: &str,
    accepted: &str,
    who: &str,
    message: &str,
    key: &PrivateKey,
) -> Result<String> {
    let who = format!(
        "{who} {} +0000",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs()
    );
    let payload =
        format!("tree {tree}\nparent {parent}\nauthor {who}\ncommitter {who}\n\n{message}\n");
    let signature = sign::sign(key, payload.as_bytes())?;
    let object = payload.replacen(
        "\n\n",
        &format!("\ngpgsig {}\n\n", signature.trim_end().replace('\n', "\n ")),
        1,
    );
    let commit = git.with_input(&["hash-object", "-t", "commit", "-w", "--stdin"], &object)?;
    let before = check(git, accepted, parent)?;
    if let Err(why) = admit(git, &before.commit, &commit, before.refused.as_ref()) {
        bail!("the fleet's machines would refuse this commit, so nothing changed: {why:#}");
    }
    Ok(commit)
}

#[cfg(test)]
#[path = "../../tests/trust/verify.rs"]
pub mod tests;
