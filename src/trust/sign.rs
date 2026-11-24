use crate::{
    fleet::{
        TYPES,
        machines::{self, MACHINES, Machines},
    },
    secrets, services,
    util::file,
};
use age::{secrecy::ExposeSecret, x25519};
use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};
use ssh_key::{
    HashAlg, LineEnding, PrivateKey, PublicKey, SshSig, private::Ed25519Keypair, rand_core::OsRng,
};
use std::path::{Path, PathBuf};

pub const KEY: &str = "/etc/mothership/signing";
const NAMESPACE: &str = "git";
const RECOVERY_SEED: &[u8] = b"mothership signing key\0";

pub fn machine_key() -> Result<Option<PrivateKey>> {
    read_key(Path::new(KEY))
}

pub fn require_machine_key() -> Result<PrivateKey> {
    machine_key()?.with_context(|| format!("{KEY} is missing: run `sudo mothership apply`"))
}

pub fn create_machine_key() -> Result<()> {
    create_key(Path::new(KEY))
}

pub fn create_key(path: &Path) -> Result<()> {
    let key = PrivateKey::random(&mut OsRng, ssh_key::Algorithm::Ed25519)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    file::create_private(path, key.to_openssh(LineEnding::LF)?.as_bytes())
}

pub fn read_key(path: &Path) -> Result<Option<PrivateKey>> {
    file::read_root_only(path)?
        .map(|bytes| parse_key(&bytes, path))
        .transpose()
}

pub fn parse_key(bytes: &[u8], path: &Path) -> Result<PrivateKey> {
    PrivateKey::from_openssh(bytes).map_err(|e| anyhow!("{}: {e}", path.display()))
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
pub enum Reach {
    Everything,
    Folders(Vec<PathBuf>),
    Versions,
}

pub const VERSIONS: &str = "versions";

impl Reach {
    pub fn parse(words: &[&str]) -> Result<Reach> {
        match words {
            [] => return Ok(Reach::Everything),
            [VERSIONS] => return Ok(Reach::Versions),
            _ => {}
        }
        let folders = words
            .iter()
            .map(|word| {
                let folder = PathBuf::from(word);
                let plain = folder
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_)));
                match plain {
                    true => Ok(folder),
                    false => Err(anyhow!("{word} isn't a folder inside the fleet")),
                }
            })
            .collect::<Result<_>>()?;
        Ok(Reach::Folders(folders))
    }

    pub fn covers(&self, path: &Path) -> bool {
        match self {
            Reach::Everything => true,
            Reach::Folders(folders) => folders.iter().any(|folder| path.starts_with(folder)),
            Reach::Versions => services::is_service_toml(path),
        }
    }

    pub fn holds(&self, other: &Reach) -> bool {
        match (self, other) {
            (Reach::Everything, _) => true,
            (Reach::Folders(_), Reach::Folders(theirs)) => {
                theirs.iter().all(|folder| self.covers(folder))
            }
            (Reach::Folders(_) | Reach::Versions, _) => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Signer {
    pub name: String,
    pub key: PublicKey,
    pub reach: Reach,
}

impl Signer {
    pub fn recovery(key: &str) -> Result<Signer> {
        Ok(Signer {
            name: secrets::RECOVERY.to_owned(),
            key: PublicKey::from_openssh(key).map_err(|e| anyhow!("the recovery key: {e}"))?,
            reach: Reach::Everything,
        })
    }
}

pub fn distinct(signers: &[Signer], recovery: Option<&Signer>) -> Result<()> {
    for (i, signer) in signers.iter().enumerate() {
        if let Some(other) = signers[..i].iter().find(|other| other.key == signer.key) {
            bail!(
                "{MACHINES}/ lists one signing key twice, as {} and {}",
                other.name,
                signer.name
            );
        }
        if recovery.is_some_and(|recovery| recovery.key == signer.key) {
            bail!("{MACHINES}/ lists the recovery key, as {}", signer.name);
        }
    }
    Ok(())
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
    before: &Machines,
    after: &Machines,
) -> Result<()> {
    for path in changed {
        let Some(name) = machines::named(path) else {
            if !signer.reach.covers(path) {
                bail!("{} can't sign a change to {}", signer.name, path.display());
            }
            continue;
        };
        for entry in [before.get(name), after.get(name)].into_iter().flatten() {
            if !signer
                .reach
                .covers(&Path::new(TYPES).join(&entry.device_type))
            {
                bail!(
                    "{} can't change {name}, a machine of {}",
                    signer.name,
                    entry.device_type
                );
            }
            if !entry.reaches().all(|reach| signer.reach.holds(&reach)) {
                bail!(
                    "{} can't change {name}, which signs more than it",
                    signer.name
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/trust/sign.rs"]
mod tests;
