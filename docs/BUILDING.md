# Building

**Languages:** English · [Deutsch](BUILDING.de.md) · [فارسی](BUILDING.fa.md)

How to build each variant on each OS. Every package is built **on its own platform**
— PyInstaller, dpkg and pkgbuild do not cross-compile, and Rust binaries are native
per OS/architecture.

For published downloads you usually don't need to build at all — see
[INSTALL.md](INSTALL.md) and the GitHub *Releases* page.

---

## One command per OS (recommended)

Wrapper scripts build the **Rust** binaries **and** the **Python** package for the
host OS and collect everything into `dist/<<os>>/`:

```bash
bash   scripts/build-linux.sh                 # on Debian/Ubuntu
bash   scripts/build-macos.sh                 # on macOS
powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1   # on Windows
```

Results:
- `dist/linux/`  — `backuptool`, `backuptool-gui`, `*.deb`
- `dist/macos/`  — `backuptool`, `backuptool-gui`, `backuptool.app`, `*.pkg`
- `dist/windows/` — `backuptool.exe`, `backuptool-gui.exe`, `backuptool-python.exe`

---

## Rust variant

Needs **Rust ≥ 1.78** (the committed `Cargo.lock` is format v4). Install via
<https://rustup.rs>; on an older toolchain run `rustup update stable`.

```bash
cd rust
cargo build --release --bin backuptool                       # CLI (single binary)
cargo build --release --features gui --bin backuptool-gui    # native Slint GUI
cargo test                                                   # unit tests
cargo clippy --all-targets --features gui -- -D warnings     # lints
```

On **Linux** the GUI needs system libraries:
```bash
sudo apt install -y build-essential pkg-config \
                    libfontconfig1-dev libxcb1-dev libxkbcommon-dev libwayland-dev
```

Binaries land in `rust/target/release/`. They are self-contained (no Python/Qt).

> "lock file version 4 was found, but this version of Cargo does not understand…" →
> your Cargo is older than 1.78. Run `rustup update stable`, or `rm rust/Cargo.lock`
> to regenerate a compatible lock.

---

## Python variant

The **CLI** needs only Python 3 (standard library). The **GUI** needs PySide6:
```bash
pip3 install PySide6
```

### Run from source / tests
```bash
cd python
./backuptool-portable                          # GUI (or CLI with arguments)
python3 -m unittest discover -s tests -v        # tests
pip install .                                   # dev install: backuptool, backuptool-gui
```

### Native packages
```bash
cd python
bash packaging/build-deb.sh                     # Linux  -> dist/backuptool_1.2.4_all.deb
bash packaging/build-macos.sh                   # macOS  -> dist/backuptool.app + .pkg
powershell -File packaging\build-windows.ps1    # Windows -> dist\backuptool.exe (PyInstaller)
```
The `.deb`/`.pkg` declare PySide6 as a runtime dependency (install it separately or
via `pip3 install PySide6`). The Windows `.exe` bundles PySide6.

---

## Releases via CI

Pushing a tag runs `.github/workflows/release.yml`, which builds the same artifacts
on GitHub-hosted Linux/macOS/Windows runners and attaches them to the GitHub Release:
```bash
bash scripts/bump-version.sh 1.2.4     # bump version everywhere + CHANGELOG stub
git commit -am "Bump version to 1.2.4"
git tag -a v1.2.4 -m "backuptool v1.2.4"
git push --follow-tags                 # triggers the Release workflow
```

Customize before building your own packages: author/publisher and the macOS bundle
id are in `python/pyproject.toml`, `python/packaging/*` and `rust/Cargo.toml`.
