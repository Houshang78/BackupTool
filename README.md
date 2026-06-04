# BackupTool

Cross-platform, parallel, incremental backup tool with a native GUI, selectable
encryption, and per-system backup sets — provided as **two independent
implementations** so you can build whichever you prefer.

| Implementation | Language | GUI | Encryption | Folder |
|---|---|---|---|---|
| **Python** | Python 3 | Qt6 (PySide6) | – | [`python/`](python/) |
| **Rust** | Rust | Slint (native) | AES-256-GCM / ChaCha20-Poly1305 | [`rust/`](rust/) |

Both share the same on-disk **manifest format** and concepts (incremental diff via
mtime/size or checksum, backup sets, metadata-preserving restore that also works
from exFAT). The UI is available in **English, German and Persian (فارسی)** and is
extensible by dropping a `lang/<code>.json` file.

## Quick links
- 🔧 Building: [`docs/BUILDING.md`](docs/BUILDING.md)
- 📦 Install / run without install: [`docs/INSTALL.md`](docs/INSTALL.md)
- 📚 Usage (paths, excludes, encryption, restore): [`docs/USAGE.md`](docs/USAGE.md)
- 📖 Trilingual quick guide (EN / DE / FA): [`docs/backuptool-guide.html`](docs/backuptool-guide.html)
- 🐍 Python implementation: [`python/README.md`](python/README.md)
- 🦀 Rust implementation: [`rust/README.md`](rust/README.md)

## Build at a glance
```bash
# Python (Qt6 GUI needs: pip install PySide6)
cd python && ./backuptool-portable          # GUI
python3 -m unittest discover -s tests        # tests

# Rust (single binary; GUI optional)
cd rust && cargo build --release             # core + CLI
cargo run --release --features gui --bin backuptool-gui
cargo test
```

### Build everything for your OS (one command)
Convenience wrappers that build the Rust binaries **and** the Python package and
collect all artifacts into `dist/<os>/`:
```bash
bash   scripts/build-linux.sh        # Rust CLI+GUI + .deb
bash   scripts/build-macos.sh        # Rust CLI+GUI + .app + .pkg
powershell -File scripts/build-windows.ps1   # Rust CLI+GUI + .exe
```
For published downloads, pushing a tag runs the **Release** workflow (see below),
which builds the same artifacts on GitHub-hosted runners.

## Releases
Pushing a tag builds and publishes binaries/installers for Linux, macOS and Windows
(Rust CLI + GUI archives, plus `.deb` / `.pkg` / `.exe`) via GitHub Actions:
```bash
git tag v1.1.0 && git push origin v1.1.0
```

## Which one should I use?
- **Python** — fastest to run/modify, great Qt GUI, no compiler needed.
- **Rust** — one dependency-free binary, built-in encryption, top hashing speed.

A backup is largely I/O-bound, so both reach near disk speed; Rust mainly wins on
hashing and on shipping a single self-contained binary.

## Author / project
- Author: **Houshang Pezeshkpour** <houshang@pezeshkpour.eu> · Company: **Atie**
- Repository: https://github.com/Houshang78/BackupTool

## License
GNU General Public License v3.0 or later (GPL-3.0-or-later) — see [LICENSE](LICENSE).
© Houshang Pezeshkpour (Atie).
