// SPDX-License-Identifier: GPL-3.0-or-later
//! Selectable encryption of file contents.
//!
//! - `none`              : no encryption
//! - `aes256gcm`         : AES-256-GCM (hardware-accelerated on modern CPUs)
//! - `chacha20poly1305`  : ChaCha20-Poly1305 (fast without AES hardware)
//!
//! Key derivation from a password via Argon2id (+ a random salt per backup set).
//! File blob format:  [nonce(12 bytes)] ++ [ciphertext+tag].

use anyhow::{anyhow, bail, Result};
use rand::RngCore;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cipher {
    None,
    Aes256Gcm,
    ChaCha20Poly1305,
}

impl Cipher {
    pub fn parse(s: &str) -> Result<Cipher> {
        Ok(match s.trim().to_lowercase().as_str() {
            "" | "none" => Cipher::None,
            "aes" | "aes256gcm" | "aes-256-gcm" => Cipher::Aes256Gcm,
            "chacha" | "chacha20" | "chacha20poly1305" => Cipher::ChaCha20Poly1305,
            other => bail!("Unknown encryption: {other}"),
        })
    }
    pub fn name(&self) -> &'static str {
        match self {
            Cipher::None => "none",
            Cipher::Aes256Gcm => "aes256gcm",
            Cipher::ChaCha20Poly1305 => "chacha20poly1305",
        }
    }
}

/// OWASP-ish defaults for Argon2id (m=19 MiB, t=2, p=1).
pub const DEFAULT_M_COST: u32 = 19_456;
pub const DEFAULT_T_COST: u32 = 2;
pub const DEFAULT_P_COST: u32 = 1;

pub fn random_salt() -> [u8; 16] {
    let mut s = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut s);
    s
}

pub fn derive_key(passphrase: &str, salt: &[u8], m: u32, t: u32, p: u32) -> Result<[u8; 32]> {
    use argon2::{Algorithm, Argon2, Params, Version};
    let params = Params::new(m, t, p, Some(32)).map_err(|e| anyhow!("Argon2 params: {e}"))?;
    let a2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    a2.hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| anyhow!("Argon2: {e}"))?;
    Ok(key)
}

pub fn encrypt(cipher: Cipher, key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>> {
    use aes_gcm::aead::{generic_array::GenericArray, Aead};
    let mut nonce = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let ct = match cipher {
        Cipher::Aes256Gcm => {
            use aes_gcm::{Aes256Gcm, KeyInit};
            let c = Aes256Gcm::new(GenericArray::from_slice(&key[..]));
            c.encrypt(GenericArray::from_slice(&nonce[..]), plaintext)
                .map_err(|_| anyhow!("AES encryption failed"))?
        }
        Cipher::ChaCha20Poly1305 => {
            use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
            let c = ChaCha20Poly1305::new(GenericArray::from_slice(&key[..]));
            c.encrypt(GenericArray::from_slice(&nonce[..]), plaintext)
                .map_err(|_| anyhow!("ChaCha encryption failed"))?
        }
        Cipher::None => bail!("encrypt() called with Cipher::None"),
    };
    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn decrypt(cipher: Cipher, key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>> {
    use aes_gcm::aead::{generic_array::GenericArray, Aead};
    if blob.len() < 12 {
        bail!("Encrypted blob too short");
    }
    let (nonce, ct) = blob.split_at(12);
    let pt = match cipher {
        Cipher::Aes256Gcm => {
            use aes_gcm::{Aes256Gcm, KeyInit};
            let c = Aes256Gcm::new(GenericArray::from_slice(&key[..]));
            c.decrypt(GenericArray::from_slice(nonce), ct)
                .map_err(|_| anyhow!("Decryption failed (wrong password?)"))?
        }
        Cipher::ChaCha20Poly1305 => {
            use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
            let c = ChaCha20Poly1305::new(GenericArray::from_slice(&key[..]));
            c.decrypt(GenericArray::from_slice(nonce), ct)
                .map_err(|_| anyhow!("Decryption failed (wrong password?)"))?
        }
        Cipher::None => bail!("decrypt() called with Cipher::None"),
    };
    Ok(pt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_aes() {
        let key = [7u8; 32];
        let blob = encrypt(Cipher::Aes256Gcm, &key, b"hello world").unwrap();
        assert_eq!(decrypt(Cipher::Aes256Gcm, &key, &blob).unwrap(), b"hello world");
    }

    #[test]
    fn roundtrip_chacha() {
        let key = [9u8; 32];
        let blob = encrypt(Cipher::ChaCha20Poly1305, &key, b"data").unwrap();
        assert_eq!(decrypt(Cipher::ChaCha20Poly1305, &key, &blob).unwrap(), b"data");
    }

    #[test]
    fn wrong_key_fails() {
        let blob = encrypt(Cipher::Aes256Gcm, &[1u8; 32], b"x").unwrap();
        assert!(decrypt(Cipher::Aes256Gcm, &[2u8; 32], &blob).is_err());
    }

    #[test]
    fn parse_names() {
        assert_eq!(Cipher::parse("aes256gcm").unwrap(), Cipher::Aes256Gcm);
        assert_eq!(Cipher::parse("none").unwrap(), Cipher::None);
        assert!(Cipher::parse("bogus").is_err());
    }

    #[test]
    fn derive_key_is_deterministic() {
        let salt = [3u8; 16];
        let a = derive_key("pw", &salt, DEFAULT_M_COST, DEFAULT_T_COST, DEFAULT_P_COST).unwrap();
        let b = derive_key("pw", &salt, DEFAULT_M_COST, DEFAULT_T_COST, DEFAULT_P_COST).unwrap();
        assert_eq!(a, b);
    }
}
