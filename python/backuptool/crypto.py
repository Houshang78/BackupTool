# SPDX-License-Identifier: GPL-3.0-or-later
"""Selectable encryption of file contents — byte-compatible with the Rust tool.

- ``none``              : no encryption
- ``aes256gcm``         : AES-256-GCM
- ``chacha20poly1305``  : ChaCha20-Poly1305

Key derivation uses Argon2id (Argon2 version 0x13) with a random 16-byte salt
per backup set. The on-disk blob layout is::

    [nonce (12 bytes)] ++ [ciphertext ++ 16-byte tag]

which is exactly what the Rust implementation writes, so a backup encrypted by
either tool can be decrypted by the other (same cipher, same KDF parameters,
both stored in the manifest).

Requires ``cryptography`` (AEAD) and ``argon2-cffi`` (Argon2id). Install with::

    pip install backuptool[crypto]      # or: pip install cryptography argon2-cffi
"""
from __future__ import annotations

import os

CIPHER_NONE = "none"
CIPHER_AES = "aes256gcm"
CIPHER_CHACHA = "chacha20poly1305"

# OWASP-ish Argon2id defaults — identical to the Rust crate's constants.
DEFAULT_M_COST = 19_456   # KiB
DEFAULT_T_COST = 2
DEFAULT_P_COST = 1

# Argon2 version 0x13 (= 19), matching argon2::Version::V0x13 in Rust.
_ARGON2_VERSION = 19


class CryptoUnavailable(RuntimeError):
    """Raised when encryption is requested but the optional libraries are missing."""


def _require_libs():
    try:
        from cryptography.hazmat.primitives.ciphers.aead import (  # noqa: F401
            AESGCM, ChaCha20Poly1305)
        from argon2.low_level import hash_secret_raw, Type  # noqa: F401
    except ImportError as e:
        raise CryptoUnavailable(
            "encryption needs the 'cryptography' and 'argon2-cffi' packages "
            "(pip install backuptool[crypto])"
        ) from e


def ensure_available() -> None:
    """Raise CryptoUnavailable if the optional encryption libraries are missing."""
    _require_libs()


def normalize_cipher(s: str) -> str:
    """Parse a user-supplied cipher name into the canonical manifest value."""
    s = (s or "none").strip().lower()
    if s in ("", "none"):
        return CIPHER_NONE
    if s in ("aes", "aes256gcm", "aes-256-gcm"):
        return CIPHER_AES
    if s in ("chacha", "chacha20", "chacha20poly1305"):
        return CIPHER_CHACHA
    raise ValueError(f"Unknown encryption: {s}")


def random_salt() -> bytes:
    return os.urandom(16)


def derive_key(passphrase: str, salt: bytes,
               m: int = DEFAULT_M_COST, t: int = DEFAULT_T_COST,
               p: int = DEFAULT_P_COST) -> bytes:
    """Derive a 32-byte key with Argon2id — identical output to the Rust tool
    for the same passphrase, salt and parameters."""
    _require_libs()
    from argon2.low_level import hash_secret_raw, Type
    return hash_secret_raw(
        secret=passphrase.encode("utf-8"),
        salt=salt,
        time_cost=t,
        memory_cost=m,
        parallelism=p,
        hash_len=32,
        type=Type.ID,
        version=_ARGON2_VERSION,
    )


def _aead(cipher: str, key: bytes):
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM, ChaCha20Poly1305
    if cipher == CIPHER_AES:
        return AESGCM(key)
    if cipher == CIPHER_CHACHA:
        return ChaCha20Poly1305(key)
    raise ValueError("encrypt/decrypt called with cipher 'none'")


def encrypt(cipher: str, key: bytes, plaintext: bytes) -> bytes:
    """Return nonce(12) ++ ciphertext ++ tag(16)."""
    _require_libs()
    nonce = os.urandom(12)
    ct = _aead(cipher, key).encrypt(nonce, plaintext, None)
    return nonce + ct


def decrypt(cipher: str, key: bytes, blob: bytes) -> bytes:
    _require_libs()
    if len(blob) < 12:
        raise ValueError("Encrypted blob too short")
    nonce, ct = blob[:12], blob[12:]
    return _aead(cipher, key).decrypt(nonce, ct, None)
