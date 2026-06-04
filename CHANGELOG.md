# Changelog

All notable changes to this project. Versions follow [Semantic Versioning](https://semver.org).

## [1.2.1]
### Added
- **Application icons** for all installers: a per-OS line-art logo (a save/backup
  floppy with the OS marker — Tux / Apple / Windows — laid where the upload arrow
  points). Sources and a reproducible generator live in `assets/logos/`
  (`make-logos.py`). Wired into packaging:
  - Linux `.deb`: hicolor PNG set (16–512 px) + `Icon=backuptool` in the desktop entry.
  - Windows: multi-resolution `.ico` (Setup icon, shortcut icon, PyInstaller `--icon`).
  - macOS: `.icns` in the `.app` bundle (`CFBundleIconFile`).
- **Clear variant choice in releases**: Rust archives are now suffixed `-rust`
  and the GitHub release notes explain which file to grab for the **Rust**
  (single binary, built-in encryption) vs **Python** (native installer, Qt GUI)
  variant, per OS.

### Fixed
- `.deb` was uninstallable where `python3-pyside6` is not in the apt repos: it
  was a hard `Depends`. PySide6 is only needed for the GUI, so it is now a
  `Recommends` (auto-installed where available, skipped otherwise) and the only
  hard dependency is `python3`. The CLI works with the standard library alone.

## [1.2.0]
### Added
- **Directory tracking** (both implementations): directories are now recorded in
  the manifest (new `dir` entry type), so **empty directories survive** a
  backup/restore round-trip and directory **permissions/owner/mtime are
  reapplied** on restore (deepest-first, so child writes can't clobber a dir's
  mtime). The manifest format stays cross-compatible between Python and Rust.
- **Streamed encryption** (Rust): files are now encrypted/decrypted **chunk by
  chunk** (1 MiB chunks) instead of being held whole in memory, so very large
  files no longer risk OOM. Each chunk binds its index and a final-chunk flag as
  AEAD associated data (reorder/truncation detection). Older whole-file encrypted
  backups remain restorable (format auto-detected via a magic header).
- **Restore-to-`/` confirmation** in the Rust (Slint) GUI, matching the Python
  GUI (skipped for dry runs).

### Fixed
- Rust GUI now right-aligns free text for **RTL languages** (Persian); Slint has
  no runtime widget mirroring, so full layout mirroring is still unavailable.
- Python default exclude `lost+found` never matched (missing `*/` prefix); added.
- Rust `scan` now **de-duplicates overlapping sources** (e.g. `/home` and
  `/home/user`), avoiding double copy work and concurrent writes to one path.
- **Path-traversal guard** on restore: manifest entries that are absolute or
  contain `..` are skipped instead of being written outside the target.
- `list` no longer counts directories in the per-set **FILES** column.
- Python manifest history no longer overwrites a prior copy when two runs land in
  the same second.

## [1.1.0]
### Added
- **System-directory backup**: curated system dirs (`/etc`, `/usr/local/etc`, `/opt`,
  plus `/srv`, `/root`, `/var/spool/cron` on Linux) are included **automatically when
  run as root**; control with `--system` / `--no-system` (both CLIs and GUIs).
- **Per-run dated logs**: every backup writes
  `<set>/.backuptool-logs/backup-<date>.log` listing the full path of each
  changed/new (`CHANGED`) and removed (`DELETED`) file plus a summary.
- **Restore tab** in the Rust (Slint) GUI.
- Per-OS build wrapper scripts (`scripts/build-*`) and a **Release** CI workflow.
- **Trilingual documentation** (EN/DE/FA) for Building, Installing and Usage.
- Documented Rust 1.78 MSRV; clearer build-script hints for old Cargo.

## [1.0.0]
### Added
- Initial release with two independent implementations: **Python (Qt6)** and
  **Rust (Slint)**, sharing one manifest format.
- Parallel, incremental backups (mtime/size or checksum); per-system backup sets.
- Selectable encryption in the Rust variant (AES-256-GCM / ChaCha20-Poly1305).
- exFAT-safe restore (permissions/owner reapplied from the manifest).
- Multilingual GUI (EN/DE/FA), tests, GitHub Actions CI, GPL-3.0 with SPDX headers.
