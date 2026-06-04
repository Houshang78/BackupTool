# backuptool (Rust)

Parallel, incremental backup tool with **selectable encryption**. A single
dependency-free binary, no Python/Qt required. Optional native **Slint** GUI.

> Status: **compiled & tested on macOS** (Rust 1.96). Core+CLI and the Slint GUI
> build cleanly; `cargo test` passes (crypto round-trips, incremental diff). Build
> on each target OS for a native binary (Rust binaries are not portable across OSes).

## Features
- **Multicore:** scan/hash/copy in parallel via `rayon` (no GIL).
- **Incremental:** compare by **mtime + size** (fast) or **BLAKE3** (`-c`).
- **Backup sets:** one set per system (own name), own manifest.
- **Permissions/owner in the manifest** → restorable, even from exFAT.
- **Selectable encryption:** `none`, **AES-256-GCM**, **ChaCha20-Poly1305**;
  key derived from a password with Argon2id (random salt per set).
- **GUI (Slint, native)** with a language selector (English / Deutsch / فارسی,
  extensible via `lang/*.json`) and an encryption dropdown.

## Build & run
Requires **Rust ≥ 1.78** (the committed `Cargo.lock` uses format v4). On an older
toolchain run `rustup update stable`, or delete `Cargo.lock` to regenerate it.
```bash
# toolchain (if needed):  https://rustup.rs
cargo build --release                                   # core + CLI
cargo run --release --features gui --bin backuptool-gui # native Slint GUI
cargo test                                              # unit tests
```
On Linux the GUI needs system libraries:
```bash
sudo apt install -y build-essential pkg-config libfontconfig1-dev \
                    libxcb1-dev libxkbcommon-dev libwayland-dev
```

## CLI
```bash
backuptool backup ~/Documents -d /Volumes/Backup -s my-laptop      # set = name
backuptool backup ~/ -d /Volumes/Backup -c -j 8                    # BLAKE3, 8 workers
backuptool backup ~/ -d /Volumes/Backup --cipher aes256gcm         # encrypted (prompts)
backuptool list    -d /Volumes/Backup
sudo backuptool restore -S /Volumes/Backup -s my-laptop -t /
```
For automation, the password can come from `BACKUPTOOL_PASSWORD`:
```bash
BACKUPTOOL_PASSWORD='secret' backuptool backup ~/ -d /Volumes/Backup --cipher aes256gcm
```

| Flag | Meaning |
|---|---|
| `-d` | destination / backup drive |
| `-s` | backup set (default: hostname) |
| `-c` | BLAKE3 compare instead of mtime/size |
| `-j N` | workers (default: CPU cores) |
| `-e` | exclude pattern, e.g. `'**/node_modules/**'` |
| `--delete` | mirror deletions in the destination |
| `--cipher` | `none` \| `aes256gcm` \| `chacha20poly1305` |
| `-n` | dry run |

## Encryption details
- **AES-256-GCM**: hardware-accelerated on modern CPUs (AES-NI), very fast.
- **ChaCha20-Poly1305**: fast even without AES hardware (e.g. older ARM).
- **Argon2id** derives the 256-bit key (m=19 MiB, t=2, p=1). The salt is in the
  manifest; each file gets a random nonce, blob format `[nonce(12)] ++ [ciphertext+tag]`.
  The manifest itself stays unencrypted (paths/metadata, no content).
- *PoC limitation:* files are read fully into memory for (de)cryption; switch to
  streamed AEAD (`aead::stream`) for very large files.

## Layout
```
src/
  manifest.rs       # serde structures, load/save
  crypto.rs         # cipher selection, Argon2id, encrypt/decrypt  (+ tests)
  engine.rs         # scan, diff, parallel copy/encrypt, restore   (+ tests)
  i18n.rs           # embedded EN/DE/FA catalogs + runtime lang/*.json
  bin/backuptool.rs # CLI (clap, indicatif)
  bin/gui.rs        # Slint GUI (background thread, progress/log)
ui/app.slint        # native UI incl. language + encryption dropdown
lang/               # en.json / de.json / fa.json
```

## Author / project
- Author: **Houshang Pezeshkpour** <houshang@pezeshkpour.eu> · Company: **Atie**
- Repository: https://github.com/Houshang78/BackupTool

## License
GNU General Public License v3.0 or later (GPL-3.0-or-later) – see [LICENSE](LICENSE).
© Houshang Pezeshkpour (Atie).
