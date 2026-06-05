// SPDX-License-Identifier: GPL-3.0-or-later
//! backuptool – core library (cross-platform, parallel, incremental).
//!
//! Modules:
//! - [`manifest`]: file metadata and manifest (JSON, serde)
//! - [`crypto`]:   selectable encryption (none / AES-256-GCM / ChaCha20-Poly1305)
//! - [`engine`]:   scan, diff, parallel copy/encrypt, restore

pub mod manifest;
pub mod crypto;
pub mod engine;
pub mod discover;
pub mod dbdump;
pub mod i18n;
