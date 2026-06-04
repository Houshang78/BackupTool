# Bauen

**Sprachen:** [English](BUILDING.md) · Deutsch · [فارسی](BUILDING.fa.md)

Wie man jede Variante auf jedem OS baut. Jedes Paket wird **auf seiner eigenen
Plattform** gebaut — PyInstaller, dpkg und pkgbuild cross-kompilieren nicht, und
Rust-Binaries sind nativ je OS/Architektur.

Für fertige Downloads brauchst du meist gar nicht zu bauen — siehe
[INSTALL.de.md](INSTALL.de.md) und die GitHub-*Releases*.

---

## Ein Befehl pro OS (empfohlen)

Wrapper-Skripte bauen die **Rust**-Binaries **und** das **Python**-Paket für das
jeweilige OS und legen alles in `dist/<os>/` ab:
```bash
bash   scripts/build-linux.sh                 # auf Debian/Ubuntu
bash   scripts/build-macos.sh                 # auf macOS
powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1   # auf Windows
```
Ergebnisse:
- `dist/linux/`  — `backuptool`, `backuptool-gui`, `*.deb`
- `dist/macos/`  — `backuptool`, `backuptool-gui`, `backuptool.app`, `*.pkg`
- `dist/windows/` — `backuptool.exe`, `backuptool-gui.exe`, `backuptool-python.exe`

---

## Rust-Variante

Benötigt **Rust ≥ 1.78** (die committete `Cargo.lock` ist Format v4). Installation
über <https://rustup.rs>; bei älterem Toolchain `rustup update stable`.
```bash
cd rust
cargo build --release --bin backuptool                       # CLI (eine Binary)
cargo build --release --features gui --bin backuptool-gui    # native Slint-GUI
cargo test                                                   # Unit-Tests
cargo clippy --all-targets --features gui -- -D warnings     # Lints
```
Unter **Linux** braucht die GUI System-Bibliotheken:
```bash
sudo apt install -y build-essential pkg-config \
                    libfontconfig1-dev libxcb1-dev libxkbcommon-dev libwayland-dev
```
Binaries liegen in `rust/target/release/` und sind eigenständig (kein Python/Qt).

> „lock file version 4 was found…" → dein Cargo ist älter als 1.78. `rustup update
> stable` ausführen, oder `rm rust/Cargo.lock`, um eine kompatible Lock zu erzeugen.

---

## Python-Variante

Die **CLI** braucht nur Python 3 (Standardbibliothek). Die **GUI** braucht PySide6:
```bash
pip3 install PySide6
```
### Aus dem Quellcode / Tests
```bash
cd python
./backuptool-portable                          # GUI (oder CLI mit Argumenten)
python3 -m unittest discover -s tests -v        # Tests
pip install .                                   # Dev-Install: backuptool, backuptool-gui
```
### Native Pakete
```bash
cd python
bash packaging/build-deb.sh                     # Linux  -> dist/backuptool_1.1.0_all.deb
bash packaging/build-macos.sh                   # macOS  -> dist/backuptool.app + .pkg
powershell -File packaging\build-windows.ps1    # Windows -> dist\backuptool.exe (PyInstaller)
```
`.deb`/`.pkg` setzen PySide6 als Laufzeit-Abhängigkeit voraus (separat bzw. per
`pip3 install PySide6`). Die Windows-`.exe` bündelt PySide6.

---

## Releases via CI

Ein Tag-Push startet `.github/workflows/release.yml`, baut dieselben Artefakte auf
GitHubs Linux/macOS/Windows-Runnern und hängt sie ans GitHub-Release:
```bash
git tag v1.1.0
git push origin v1.1.0
```
Vor eigenen Paketen anpassen: Autor/Publisher und die macOS-Bundle-ID in
`python/pyproject.toml`, `python/packaging/*` und `rust/Cargo.toml`.
