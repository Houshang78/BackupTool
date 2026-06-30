// SPDX-License-Identifier: GPL-3.0-or-later
//! Manifest: metadata for every file + KDF parameters. Compatible with the
//! Python idea (path -> size/mtime/permissions/owner), extended with encryption
//! information.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const MANIFEST_NAME: &str = ".backuptool-manifest.json";

/// Entry kind. Serializes as the lowercase strings "file" / "symlink",
/// keeping the manifest format identical across the Python and Rust tools.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    File,
    Symlink,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FileMeta {
    pub size: u64,
    pub mtime: i64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    #[serde(rename = "type")]
    pub kind: Kind,
    #[serde(default)]
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blake3: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct KdfParams {
    pub algo: String, // "argon2id"
    pub salt_b64: String,
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

#[derive(Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub set: String,
    pub host: String,
    pub created: String,
    pub use_checksum: bool,
    pub cipher: String, // "none" | "aes256gcm" | "chacha20poly1305"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kdf: Option<KdfParams>,
    pub files: BTreeMap<String, FileMeta>,
}

pub fn load(path: &Path) -> Option<Manifest> {
    let data = std::fs::read(path).ok()?;
    serde_json::from_slice(&data).ok()
}

pub fn save(path: &Path, m: &Manifest) -> anyhow::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(m)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}
