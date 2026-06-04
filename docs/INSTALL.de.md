# Installieren & Starten

**Sprachen:** [English](INSTALL.md) · Deutsch · [فارسی](INSTALL.fa.md)

Zwei Wege: **ohne Installation starten** (portabel) oder ein **natives Paket
installieren**. Danach zeigt [USAGE.de.md](USAGE.de.md), wie man Backups macht.

---

## A. Ohne Installation starten (portabel)

### Rust-Binary (eine Datei, keine Abhängigkeiten)
Nach `cargo build --release` (siehe [BUILDING.de.md](BUILDING.de.md)) oder aus einem
Release-Archiv:
```bash
# aus einem Build
./rust/target/release/backuptool --help
./rust/target/release/backuptool-gui            # native GUI
# aus einem Release-Archiv
tar -xzf backuptool-v1.1.0-linux-x86_64.tar.gz  # oder Windows-.zip entpacken
./backuptool --help
./backuptool-gui
```
Die Binary lässt sich überallhin kopieren (auch auf die Sicherungsplatte) und direkt
starten.

### Python-Portable-Starter
Die CLI braucht nur Python 3; die GUI zusätzlich `pip3 install PySide6`.
```bash
cd python
./backuptool-portable                                   # GUI
./backuptool-portable backup ~/Documents -d /Volumes/Backup -s mein-laptop --progress
```
`backuptool-portable` setzt `PYTHONPATH` selbst und ruft `python3 -m backuptool`. Du
kannst den ganzen `python/`-Ordner auf die Platte kopieren und von dort starten.

Unter Windows ist der PyInstaller-Build eine einzige portable Datei — einfach
`backuptool.exe` starten (Doppelklick = GUI, mit Argumenten = CLI).

---

## B. Ein natives Paket installieren

### Linux (.deb)
```bash
sudo apt install ./backuptool_1.1.0_all.deb
backuptool --help          # CLI
backuptool gui             # GUI (oder Menüeintrag „backuptool")
```
Braucht `python3-pyside6` für die GUI (`sudo apt install python3-pyside6` oder
`pip3 install PySide6`). Deinstallieren: `sudo apt remove backuptool`.

### macOS (.pkg oder .app)
```bash
sudo installer -pkg backuptool-1.1.0.pkg -target /     # CLI nach /usr/local/bin
backuptool --help
```
Oder `backuptool.app` doppelklicken (GUI). In beiden Fällen einmalig:
`pip3 install PySide6`. Deinstallieren: `/usr/local/bin/backuptool` und
`/usr/local/lib/backuptool` entfernen (und die App, falls nach /Applications kopiert).

### Windows (.exe / Setup)
- Portabel: einfach `backuptool.exe` behalten und starten.
- Installer: `backuptool-setup-1.1.0.exe` (Inno Setup) ausführen — bietet
  Startmenü-Eintrag, optionales Desktop-Symbol und optionales Hinzufügen zur `PATH`.
  Deinstallieren über *Einstellungen → Apps*.

---

## Welche Binary macht was?

| Name | Variante | Einsatz |
|---|---|---|
| `backuptool` | CLI | Skripte / Cron / SSH |
| `backuptool-gui` (Rust) / `backuptool gui` (Python) | GUI | per Maus |
| `backuptool.exe` / `backuptool-python.exe` | Windows | Rust- / Python-Build |

---

## Erster Lauf

```bash
backuptool --version
backuptool list -d /Volumes/Backup       # leer, bis das erste Backup läuft
# sicheres erstes Backup (Probelauf)
backuptool backup ~/Documents -d /Volumes/Backup -s mein-laptop -n
```
Weiter: [USAGE.de.md](USAGE.de.md) — Pfade, Ausschlüsse, Verschlüsselung,
inkrementell und Restore.
