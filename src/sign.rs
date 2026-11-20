use crate::{file, secrets};
use age::{secrecy::ExposeSecret, x25519};
use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};
use ssh_key::{
    HashAlg, LineEnding, PrivateKey, PublicKey, SshSig, private::Ed25519Keypair, rand_core::OsRng,
};
use std::path::{Path, PathBuf};

pub const KEY: &str = "/etc/mothership/signing";
pub const SIGNERS: &str = "signers.txt";
const NAMESPACE: &str = "git";
const RECOVERY_SEED: &[u8] = b"mothership signing key\0";

pub fn machine_key() -> Result<Option<PrivateKey>> {
    let path = Path::new(KEY);
    file::read_root_only(path)?
        .map(|bytes| PrivateKey::from_openssh(bytes).map_err(|e| anyhow!("{KEY}: {e}")))
        .transpose()
}

pub fn require_machine_key() -> Result<PrivateKey> {
    machine_key()?.with_context(|| format!("{KEY} is missing: run `mothership key`"))
}

pub fn create_machine_key() -> Result<()> {
    let key = PrivateKey::random(&mut OsRng, ssh_key::Algorithm::Ed25519)?;
    file::create_private(Path::new(KEY), key.to_openssh(LineEnding::LF)?.as_bytes())
}

pub fn recovery_key(age: &x25519::Identity) -> PrivateKey {
    let mut seed = Sha256::new();
    seed.update(RECOVERY_SEED);
    seed.update(age.to_string().expose_secret().as_bytes());
    let keypair = Ed25519Keypair::from_seed(&seed.finalize().into());
    PrivateKey::from(keypair)
}

pub fn public(key: &PrivateKey) -> Result<String> {
    Ok(key.public_key().to_openssh()?)
}

pub fn sign(key: &PrivateKey, payload: &[u8]) -> Result<String> {
    Ok(key
        .sign(NAMESPACE, HashAlg::Sha512, payload)?
        .to_pem(LineEnding::LF)?)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Signer {
    pub name: String,
    pub key: PublicKey,
    pub scope: Option<PathBuf>,
}

impl Signer {
    pub fn recovery(key: &str) -> Result<Signer> {
        Ok(Signer {
            name: secrets::RECOVERY.to_owned(),
            key: PublicKey::from_openssh(key).map_err(|e| anyhow!("the recovery key: {e}"))?,
            scope: None,
        })
    }

    fn covers(&self, path: &Path) -> bool {
        self.scope
            .as_ref()
            .is_none_or(|scope| path.starts_with(scope))
    }

    fn may_hand_out(&self, other: &Signer) -> bool {
        match (&self.scope, &other.scope) {
            (None, _) => true,
            (Some(mine), Some(theirs)) => theirs.starts_with(mine),
            (Some(_), None) => false,
        }
    }
}

pub fn parse_signers(text: &str) -> Result<Vec<Signer>> {
    secrets::parse(text)?
        .into_iter()
        .map(|entry| {
            let mut words = entry.key.split_whitespace();
            let (Some(kind), Some(data)) = (words.next(), words.next()) else {
                bail!("{}'s key isn't an SSH public key", entry.name);
            };
            let key = PublicKey::from_openssh(&format!("{kind} {data}"))
                .map_err(|e| anyhow!("{}'s key: {e}", entry.name))?;
            let scope = words.next().map(PathBuf::from);
            if let Some(extra) = words.next() {
                bail!("{}'s line ends with {extra}, after its scope", entry.name);
            }
            Ok(Signer {
                name: entry.name,
                key,
                scope,
            })
        })
        .collect()
}

pub struct Commit {
    pub payload: Vec<u8>,
    pub signature: Option<SshSig>,
}

impl Commit {
    pub fn parse(object: &[u8]) -> Result<Commit> {
        let text = std::str::from_utf8(object).context("a commit that isn't UTF-8")?;
        let mut payload = String::with_capacity(text.len());
        let mut armored: Option<String> = None;
        let mut in_signature = false;
        let mut in_headers = true;
        for line in text.split_inclusive('\n') {
            if in_headers && in_signature {
                if let Some(rest) = line.strip_prefix(' ') {
                    armored.get_or_insert_default().push_str(rest);
                    continue;
                }
                in_signature = false;
            }
            if in_headers && let Some(rest) = line.strip_prefix("gpgsig ") {
                if armored.is_some() {
                    bail!("a commit with two signatures");
                }
                armored = Some(rest.to_owned());
                in_signature = true;
                continue;
            }
            if line == "\n" {
                in_headers = false;
            }
            payload.push_str(line);
        }
        let signature = armored
            .map(|text| {
                SshSig::from_pem(text.trim_end()).map_err(|e| anyhow!("the signature: {e}"))
            })
            .transpose()?;
        Ok(Commit {
            payload: payload.into_bytes(),
            signature,
        })
    }

    pub fn signer<'a>(&self, signers: &'a [Signer]) -> Option<&'a Signer> {
        let signature = self.signature.as_ref()?;
        signers.iter().find(|signer| {
            signer
                .key
                .verify(NAMESPACE, &self.payload, signature)
                .is_ok()
        })
    }
}

pub fn allowed(
    signer: &Signer,
    changed: &[PathBuf],
    before: &[Signer],
    after: &[Signer],
) -> Result<()> {
    for path in changed {
        if path == Path::new(SIGNERS) {
            let moved = before
                .iter()
                .filter(|s| !after.contains(s))
                .chain(after.iter().filter(|s| !before.contains(s)));
            for other in moved {
                if !signer.may_hand_out(other) {
                    bail!(
                        "{} can't change {}'s line in {SIGNERS}",
                        signer.name,
                        other.name
                    );
                }
            }
        } else if !signer.covers(path) {
            bail!("{} can't sign a change to {}", signer.name, path.display());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/sign.rs"]
mod tests;
