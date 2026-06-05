# Installing & Running

**Languages:** English · [Deutsch](INSTALL.de.md) · [فارسی](INSTALL.fa.md)

Two ways to use the tool: **run without installing** (portable) or **install** a
native package. After that, see [USAGE.md](USAGE.md) for how to take backups.

---

## A. Run without installing (portable)

### Rust binary (single file, no dependencies)
After `cargo build --release` (see [BUILDING.md](BUILDING.md)) or after downloading a
release archive:
```bash
# from a build
./rust/target/release/backuptool --help
./rust/target/release/backuptool-gui            # native GUI

# from a release archive
tar -xzf backuptool-v1.2.4-linux-x86_64.tar.gz  # or unzip the Windows .zip
./backuptool --help
./backuptool-gui
```
Copy the binary anywhere (including onto the backup drive) and run it directly.

### Python portable launcher
The CLI needs only Python 3; the GUI also needs `pip3 install PySide6`.
```bash
cd python
./backuptool-portable                                   # GUI
./backuptool-portable backup ~/Documents -d /Volumes/Backup -s my-laptop --progress
```
`backuptool-portable` sets `PYTHONPATH` itself and calls `python3 -m backuptool`. You
can copy the whole `python/` folder onto the backup drive and run it from there.

On Windows, the PyInstaller build is a single portable file — just run
`backuptool.exe` (double-click for the GUI, or pass arguments for the CLI).

---

## B. Install a native package

### Linux (.deb)
```bash
sudo apt install ./backuptool_1.2.4_all.deb
backuptool --help          # CLI
backuptool gui             # GUI (or the "backuptool" menu entry)
```
Needs `python3-pyside6` for the GUI (`sudo apt install python3-pyside6`, or
`pip3 install PySide6`). Uninstall: `sudo apt remove backuptool`.

### macOS (.pkg or .app)
```bash
sudo installer -pkg backuptool-1.2.4.pkg -target /     # CLI into /usr/local/bin
backuptool --help
```
Or double-click `backuptool.app` for the GUI. Either way, once:
`pip3 install PySide6`. Uninstall: remove `/usr/local/bin/backuptool` and
`/usr/local/lib/backuptool` (and the app from /Applications if you copied it there).

### Windows (.exe / setup)
- Portable: just keep and run `backuptool.exe`.
- Installer: run `backuptool-setup-1.2.4.exe` (Inno Setup). It offers a Start-menu
  entry, an optional desktop icon, and optionally adds the CLI to `PATH`.
  Uninstall via *Settings → Apps*.

---

## Which binary does what?

| Name | Variant | Use |
|---|---|---|
| `backuptool` | CLI | scripting / cron / SSH |
| `backuptool-gui` (Rust) / `backuptool gui` (Python) | GUI | point-and-click |
| `backuptool.exe` / `backuptool-python.exe` | Windows | Rust / Python builds |

---

## First run

```bash
# verify it works
backuptool --version
backuptool list -d /Volumes/Backup       # empty until you make a backup

# a safe first backup (dry run, see what would happen)
backuptool backup ~/Documents -d /Volumes/Backup -s my-laptop -n
```

Next: [USAGE.md](USAGE.md) — paths, excludes, encryption, incremental and restore.
