use md5::Md5;
use sha2::{Digest, Sha256};

pub fn md5(bytes: &[u8]) -> String {
    hex(&Md5::digest(bytes))
}

pub fn hex(digest: &[u8]) -> String {
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
