# backuptool (Python)

Cross-platform, **parallel**, **incremental** backup tool for **Linux, macOS and
Windows** with a multilingual Qt6 GUI.

- **Multicore:** parallel copying (threads) and SHA-256 compare (processes).
- **Incremental:** only changed files are copied — compared by **mtime + size** or
  optionally **SHA-256** (`-c`).
- **Multiple systems:** one **backup set** per system (own name / hostname) in the
  same destination.
- **Permissions/owner preserved** — even on **exFAT**: the metadata (mode, uid/gid,
  mtime) live in the manifest and are reapplied on restore. No tar needed.
- **GUI (Qt6)** with a language selector (English / Deutsch / فارسی, extensible),
  selectable storage location and set name, plus a full command line.
- **Portable:** run straight from the backup drive, or install as **.deb** (Linux),
  **.pkg / .app** (macOS) or **.exe / setup** (Windows).

> Looking for a single dependency-free binary and built-in encryption? See the
> Rust implementation in the `rust/` folder.

## Quick start (portable, no install)

```bash
./backuptool-portable                       # GUI (needs PySide6, see below)
./backuptool-portable backup ~/Documents -d /Volumes/Backup -s my-laptop --progress
```

### GUI dependency
```bash
pip3 install PySide6        # once, for the graphical interface
```
The **command line** runs without PySide6 (standard library only).

## Command line

```bash
backuptool backup ~/ -d /Volumes/Backup --progress              # set = hostname
backuptool backup ~/ -d /Volumes/Backup -s workstation-01       # own set name
backuptool backup ~/ -d /Volumes/Backup -c -j 8                 # SHA-256, 8 workers
backuptool backup ~/ -d /Volumes/Backup -n -e '*/node_modules/*' --delete
backuptool list    -d /Volumes/Backup
sudo backuptool restore -S /Volumes/Backup -s workstation-01 -t / --progress
```

| Command | Flag | Meaning |
|---|---|---|
| backup | `-d` | destination / backup drive (required) |
| backup | `-s` | backup set name (default: hostname) |
| backup | `-c` | compare by SHA-256 (multicore) instead of mtime/size |
| backup | `-j N` | worker count (default: CPU cores) |
| backup | `-e` | exclude pattern (repeatable) |
| backup | `--delete` | delete in destination what was removed from source |
| backup/restore | `-n` | dry run |
| restore | `-S / -s / -t` | backup folder / set / target root |
| restore | `--no-meta` | do NOT reapply permissions/owner |

## How the incremental compare works

Each set has a manifest `<set>/.backuptool-manifest.json` with path, size, mtime,
mode and uid/gid (and optionally SHA-256) per file. On the next run a file counts as
changed when **size or mtime** differ (default) or when the **SHA-256** differs
(`-c`). Only changed/new files are copied; each run also keeps a manifest copy under
`.backuptool-history/`.

## exFAT / cross-platform targets

exFAT (readable/writable on Linux, macOS, Windows) cannot store Unix permissions.
That is fine here: the metadata is in the **manifest** and reapplied on **restore**
(`chmod`/`utime`; `chown` as root). Symlinks that exFAT cannot store stay recorded in
the manifest and are recreated on restore on a capable filesystem.

## Packaging

```bash
bash packaging/build-deb.sh        # Linux  -> dist/backuptool_1.6.0_all.deb
bash packaging/build-macos.sh      # macOS  -> backuptool.app + .pkg
powershell -File packaging\build-windows.ps1   # Windows -> dist\backuptool.exe
pip install .                      # dev install (entry points: backuptool, backuptool-gui)
```
Each package is built **on its own platform** (PyInstaller/dpkg/pkgbuild do not
cross-compile).

## Tests
```bash
python3 -m unittest discover -s tests -v
```

## Languages
The GUI loads JSON catalogs from `backuptool/lang/`. Add a language by dropping a
`<code>.json` file there (e.g. `fr.json`) — it is picked up automatically.

## Author / project
- Author: **Houshang Pezeshkpour** <houshang@pezeshkpour.eu> · Company: **Atie**
- Repository: https://github.com/Houshang78/BackupTool

## License
GNU General Public License v3.0 or later (GPL-3.0-or-later) – see [LICENSE](LICENSE).
© Houshang Pezeshkpour (Atie).
