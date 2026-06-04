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

// ---------------------------------------------------------------------------
// Streamed (chunked) AEAD — encrypts/decrypts without holding the whole file in
// memory. Each chunk is sealed independently with a fresh random nonce; the
// chunk index and a "final" flag are bound as associated data so chunks cannot
// be reordered, duplicated or truncated without detection.
//
// On-disk layout:
//   MAGIC(6)  then per chunk: [flag:u8][ct_len:u32 LE][nonce:12][ciphertext]
//   flag = 1 marks the last chunk. ct_len includes the 16-byte tag.
// ---------------------------------------------------------------------------
use std::io::{Read, Write};

pub const STREAM_MAGIC: &[u8; 6] = b"BTSC1\0";
const CHUNK: usize = 1 << 20; // 1 MiB plaintext per chunk

fn aad_of(index: u64, flag: u8) -> [u8; 9] {
    let mut a = [0u8; 9];
    a[..8].copy_from_slice(&index.to_le_bytes());
    a[8] = flag;
    a
}

fn seal(cipher: Cipher, key: &[u8; 32], nonce: &[u8; 12], pt: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    use aes_gcm::aead::{generic_array::GenericArray, Aead, Payload};
    let payload = Payload { msg: pt, aad };
    Ok(match cipher {
        Cipher::Aes256Gcm => {
            use aes_gcm::{Aes256Gcm, KeyInit};
            Aes256Gcm::new(GenericArray::from_slice(&key[..]))
                .encrypt(GenericArray::from_slice(&nonce[..]), payload)
                .map_err(|_| anyhow!("AES encryption failed"))?
        }
        Cipher::ChaCha20Poly1305 => {
            use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
            ChaCha20Poly1305::new(GenericArray::from_slice(&key[..]))
                .encrypt(GenericArray::from_slice(&nonce[..]), payload)
                .map_err(|_| anyhow!("ChaCha encryption failed"))?
        }
        Cipher::None => bail!("seal() called with Cipher::None"),
    })
}

fn open(cipher: Cipher, key: &[u8; 32], nonce: &[u8; 12], ct: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    use aes_gcm::aead::{generic_array::GenericArray, Aead, Payload};
    let payload = Payload { msg: ct, aad };
    Ok(match cipher {
        Cipher::Aes256Gcm => {
            use aes_gcm::{Aes256Gcm, KeyInit};
            Aes256Gcm::new(GenericArray::from_slice(&key[..]))
                .decrypt(GenericArray::from_slice(&nonce[..]), payload)
                .map_err(|_| anyhow!("Decryption failed (wrong password or corrupt data?)"))?
        }
        Cipher::ChaCha20Poly1305 => {
            use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
            ChaCha20Poly1305::new(GenericArray::from_slice(&key[..]))
                .decrypt(GenericArray::from_slice(&nonce[..]), payload)
                .map_err(|_| anyhow!("Decryption failed (wrong password or corrupt data?)"))?
        }
        Cipher::None => bail!("open() called with Cipher::None"),
    })
}

/// Read up to `buf.len()` bytes, returning fewer only at end of input.
fn fill(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        match r.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(got)
}

pub fn encrypt_stream<R: Read, W: Write>(cipher: Cipher, key: &[u8; 32], mut r: R, mut w: W) -> Result<()> {
    w.write_all(STREAM_MAGIC)?;
    let mut cur = vec![0u8; CHUNK];
    let mut nread = fill(&mut r, &mut cur)?;
    let mut index: u64 = 0;
    loop {
        // One-chunk lookahead so we can flag the true final chunk even when the
        // file size is an exact multiple of CHUNK.
        let mut next = vec![0u8; CHUNK];
        let n_next = if nread == CHUNK { fill(&mut r, &mut next)? } else { 0 };
        let last = n_next == 0;
        let flag: u8 = if last { 1 } else { 0 };
        let mut nonce = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let ct = seal(cipher, key, &nonce, &cur[..nread], &aad_of(index, flag))?;
        w.write_all(&[flag])?;
        w.write_all(&(ct.len() as u32).to_le_bytes())?;
        w.write_all(&nonce)?;
        w.write_all(&ct)?;
        if last {
            break;
        }
        cur = next;
        nread = n_next;
        index += 1;
    }
    w.flush()?;
    Ok(())
}

pub fn decrypt_stream<R: Read, W: Write>(cipher: Cipher, key: &[u8; 32], mut r: R, mut w: W) -> Result<()> {
    let mut magic = [0u8; 6];
    if fill(&mut r, &mut magic)? != 6 || &magic != STREAM_MAGIC {
        bail!("Not a streamed-encryption blob");
    }
    let mut index: u64 = 0;
    loop {
        let mut flag = [0u8; 1];
        if fill(&mut r, &mut flag)? != 1 {
            bail!("Encrypted data truncated (missing final chunk)");
        }
        let mut lenb = [0u8; 4];
        if fill(&mut r, &mut lenb)? != 4 {
            bail!("Encrypted data truncated");
        }
        let ct_len = u32::from_le_bytes(lenb) as usize;
        let mut nonce = [0u8; 12];
        if fill(&mut r, &mut nonce)? != 12 {
            bail!("Encrypted data truncated");
        }
        let mut ct = vec![0u8; ct_len];
        if fill(&mut r, &mut ct)? != ct_len {
            bail!("Encrypted data truncated");
        }
        let pt = open(cipher, key, &nonce, &ct, &aad_of(index, flag[0]))?;
        w.write_all(&pt)?;
        if flag[0] == 1 {
            break;
        }
        index += 1;
    }
    w.flush()?;
    Ok(())
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

    fn stream_roundtrip(cipher: Cipher, len: usize) {
        let key = [5u8; 32];
        let plain: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
        let mut enc = Vec::new();
        encrypt_stream(cipher, &key, &plain[..], &mut enc).unwrap();
        assert_eq!(&enc[..6], STREAM_MAGIC);
        let mut dec = Vec::new();
        decrypt_stream(cipher, &key, &enc[..], &mut dec).unwrap();
        assert_eq!(dec, plain);
    }

    #[test]
    fn stream_roundtrip_sizes() {
        // empty, sub-chunk, exact chunk multiple, and multi-chunk-with-remainder.
        for &len in &[0usize, 100, CHUNK, 2 * CHUNK, 2 * CHUNK + 123] {
            stream_roundtrip(Cipher::Aes256Gcm, len);
            stream_roundtrip(Cipher::ChaCha20Poly1305, len);
        }
    }

    #[test]
    fn stream_detects_truncation() {
        let key = [5u8; 32];
        let plain = vec![7u8; 3 * CHUNK];
        let mut enc = Vec::new();
        encrypt_stream(Cipher::Aes256Gcm, &key, &plain[..], &mut enc).unwrap();
        enc.truncate(enc.len() - 50); // drop part of the final chunk
        let mut out = Vec::new();
        assert!(decrypt_stream(Cipher::Aes256Gcm, &key, &enc[..], &mut out).is_err());
    }

    #[test]
    fn stream_wrong_key_fails() {
        let mut enc = Vec::new();
        encrypt_stream(Cipher::Aes256Gcm, &[1u8; 32], &b"secret"[..], &mut enc).unwrap();
        let mut out = Vec::new();
        assert!(decrypt_stream(Cipher::Aes256Gcm, &[2u8; 32], &enc[..], &mut out).is_err());
    }
}
