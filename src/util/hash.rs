use anyhow::Result;
use md5::Md5;
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path};

pub fn md5(bytes: &[u8]) -> String {
    hex(&Md5::digest(bytes))
}

pub fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

pub fn sha256_file(path: &Path) -> Result<Option<String>> {
    let Some(mut file) = crate::util::file::if_found(fs::File::open(path))? else {
        return Ok(None);
    };
    let mut digest = Sha256::new();
    let mut chunk = vec![0; 1 << 16];
    loop {
        match file.read(&mut chunk)? {
            0 => return Ok(Some(hex(&digest.finalize()))),
            read => digest.update(&chunk[..read]),
        }
    }
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub struct Fields(Sha256);

impl Fields {
    pub fn new() -> Self {
        Fields(Sha256::new())
    }

    pub fn add(&mut self, bytes: &[u8]) {
        self.0.update((bytes.len() as u64).to_le_bytes());
        self.0.update(bytes);
    }

    pub fn finish(self) -> String {
        hex(&self.0.finalize())
    }
}
