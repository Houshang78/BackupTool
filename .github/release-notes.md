## Choose your variant — Rust or Python

backuptool ships as **two independent implementations** that share the same
on-disk backup format. Pick whichever you prefer (both read each other's
unencrypted backups):

### 🦀 Rust — single self-contained binary
Built-in encryption (AES-256-GCM / ChaCha20-Poly1305), native Slint GUI,
nothing else to install.

| OS | File |
|----|------|
| Linux x86-64 | `backuptool-<tag>-linux-x86_64-rust.tar.gz` |
| macOS (Apple Silicon) | `backuptool-<tag>-macos-arm64-rust.tar.gz` |
| Windows x86-64 | `backuptool-<tag>-windows-x86_64-rust.zip` |

### 🐍 Python — native OS installer, Qt6 GUI
Needs Python 3. The CLI runs on the standard library alone; the GUI needs
PySide6 (`pip install PySide6`, or it is pulled in automatically where packaged).

| OS | File |
|----|------|
| Linux (Debian/Ubuntu) | `backuptool_<ver>_all.deb` |
| macOS | `backuptool-<ver>.pkg` (CLI) · `backuptool-<tag>-macos-app-python.tar.gz` (.app GUI) |
| Windows | `backuptool.exe` (portable single-file; build the Inno Setup installer yourself with `ISCC` if you want one) |

See `CHANGELOG.md` for what changed in this release.
