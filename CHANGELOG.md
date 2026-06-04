# Changelog

All notable changes to this project. Versions follow [Semantic Versioning](https://semver.org).

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
