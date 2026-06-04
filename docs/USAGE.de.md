# Bedienungsanleitung

**Sprachen:** [English](USAGE.md) · Deutsch · [فارسی](USAGE.fa.md)

Wie man Backups auf jede unterstützte Art erstellt — Pfade wählen, Verzeichnisse
ausschließen, mit oder ohne Verschlüsselung, inkrementeller Abgleich und
Wiederherstellung. Gilt für beide Implementierungen; **Unterschiede zwischen Python
und Rust sind markiert**.

- Python: `backuptool` (oder `./backuptool-portable`) — Prüfsummen-Abgleich mit
  **SHA-256**, **keine Verschlüsselung**.
- Rust: `backuptool` — Prüfsummen-Abgleich mit **BLAKE3**, **Verschlüsselung**
  (`--cipher`).

> In den Beispielen ist `backuptool` der Befehl. Mit dem portablen Python-Starter
> stattdessen `./backuptool-portable`, mit einer frisch gebauten Rust-Binary
> `./target/release/backuptool`.

---

## 1. Grundbegriffe

- **Backup-Set** — eine benannte Kopie eines Systems im Ziel. Name mit `-s NAME`
  (Standard: Hostname). Mehrere Systeme teilen sich eine Platte:
  ```
  /Volumes/Backup/
    mein-laptop/    .backuptool-manifest.json + Dateien
    arbeits-pc/     .backuptool-manifest.json + Dateien
  ```
- **Manifest** — `<set>/.backuptool-manifest.json` speichert Pfad, Größe, mtime,
  Modus, uid/gid (im Prüfsummen-Modus auch den Hash) jeder Datei. Es steuert den
  inkrementellen Abgleich und erlaubt beim Restore das Wiederherstellen der Rechte —
  auch von exFAT.
- **Inkrementell** — nur Geändertes/Neues wird kopiert. „Geändert", wenn **Größe oder
  mtime** abweichen (Standard) oder die **Prüfsumme** abweicht (`-c`).

---

## 2. Pfade wählen

### Quellen (was gesichert wird)
Eine oder mehrere Dateien/Ordner als Argumente:
```bash
backuptool backup ~/Documents ~/Pictures ~/.config -d /Volumes/Backup
```
Pfade werden **absolut** im Set gespeichert: `/home/me/Documents/a.txt` →
`<dest>/<set>/home/me/Documents/a.txt`. (Auf macOS lösen sich symlink-Wurzeln wie
`/tmp` zum echten Pfad auf, z. B. `/private/tmp`.)

### Ziel (`-d`)
Die Sicherungsplatte oder ein Ordner, wird bei Bedarf angelegt:
```bash
backuptool backup ~/ -d /Volumes/Backup        # externe Platte
backuptool backup ~/ -d /mnt/nas/backups       # Netzlaufwerk
```

### Set-Name (`-s`)
```bash
backuptool backup ~/ -d /Volumes/Backup -s mein-laptop
backuptool list -d /Volumes/Backup             # alle Sets auf der Platte
```

### System-Verzeichnisse (als root)
Beim Lauf als **root** (`sudo`) werden ausgewählte System-Verzeichnisse
**automatisch** einbezogen: `/etc`, `/usr/local/etc`, `/opt` (unter Linux zusätzlich
`/srv`, `/root`, `/var/spool/cron`). Sie werden unter ihren vollständigen Pfaden
gespeichert und wie alles andere inkrementell aktualisiert. Steuerung:
```bash
sudo backuptool backup ~/ -d /Volumes/Backup --system     # erzwingen (jeder Nutzer)
backuptool backup ~/ -d /Volumes/Backup --no-system       # nie einbeziehen
```
Nur lesbare Einträge werden erfasst — ohne root wird das meiste von `/etc`
übersprungen.

---

## 3. Verzeichnisse / Dateien ausschließen

`-e MUSTER` (mehrfach). Caches, Papierkorb und Thumbnails sind standardmäßig
ausgeschlossen. **Die Muster-Syntax unterscheidet sich:**

| | Python | Rust |
|---|---|---|
| Engine | `fnmatch` auf den ganzen Pfad | `globset` (`**` über Ordner) |
| Ordner überall ausschließen | `-e '*/node_modules/*'` | `-e '**/node_modules/**'` |
| Nach Endung | `-e '*.iso'` | `-e '**/*.iso'` |
| Einen Pfad | `-e '/home/me/big/*'` | `-e '/home/me/big/**'` |

```bash
# Python
backuptool backup ~/ -d /Volumes/Backup -e '*/node_modules/*' -e '*/.git/*' -e '*.iso'
# Rust
backuptool backup ~/ -d /Volumes/Backup -e '**/node_modules/**' -e '**/.git/**' -e '**/*.iso'
```
Standard-Ausschlüsse (beide): `.cache`, Papierkorb, `.thumbnails`, Browser-Caches,
`lost+found`, macOS-Reste (`.Spotlight-V100`/`.Trashes`/`.fseventsd`; Rust auch
`.DS_Store`).

---

## 4. Inkrementeller Abgleich

```bash
# Schnell (Standard): geändert bei Größe oder mtime
backuptool backup ~/ -d /Volumes/Backup
# Gründlich: geändert bei abweichendem Inhalts-Hash (alle Kerne)
backuptool backup ~/ -d /Volumes/Backup -c        # Python: SHA-256 | Rust: BLAKE3
# Spiegel: im Ziel auch löschen, was in der Quelle entfernt wurde
backuptool backup ~/ -d /Volumes/Backup --delete
# Worker (Parallelität), Standard = CPU-Kerne
backuptool backup ~/ -d /Volumes/Backup -j 8
# Probelauf: zeigt nur, schreibt nichts
backuptool backup ~/ -d /Volumes/Backup -n
```

### Logs je Lauf (mit Datum)
Jedes echte Backup schreibt ein datiertes Log nach
`<dest>/<set>/.backuptool-logs/backup-JJJJmmtt-HHMMSS.log` mit dem **vollständigen
Pfad** jeder geänderten/neuen Datei (`CHANGED`) und jeder entfernten Datei
(`DELETED`) plus Kopfzeile (Set, Host, Datum, Zahlen):
```
# backuptool  set=mein-laptop  host=server01  2026-06-04T18:13:46
# changed/new=2 unchanged=120 deleted=0 errors=0
CHANGED	/etc/hosts
CHANGED	/home/me/Documents/notes.md
```

---

## 5. Verschlüsselung — *nur Rust*

Cipher mit `--cipher` wählen. Der Inhalt jeder Datei wird verschlüsselt gespeichert;
das Manifest bleibt lesbar (nur Pfade/Metadaten). Der Schlüssel wird per Argon2id aus
dem Passwort abgeleitet (zufälliges Salt je Set).
```bash
backuptool backup ~/ -d /Volumes/Backup                          # ohne (Standard)
backuptool backup ~/ -d /Volumes/Backup --cipher aes256gcm       # fragt Passwort
backuptool backup ~/ -d /Volumes/Backup --cipher chacha20poly1305
```
Nicht-interaktiv (Automatisierung/Cron) — Passwort per Umgebungsvariable:
```bash
BACKUPTOOL_PASSWORD='dein-geheimnis' backuptool backup ~/ -d /Volumes/Backup --cipher aes256gcm
```
> Passwort sicher aufbewahren — **ohne es ist das verschlüsselte Backup nicht
> wiederherstellbar.** Cipher und Salt stehen im Manifest, daher fragt `restore`
> automatisch nach dem Passwort. Die Python-Variante verschlüsselt nicht.

---

## 6. Wiederherstellen

```bash
# Immer zuerst Probelauf in ein Staging-Verzeichnis (schreibt nichts)
backuptool restore -S /Volumes/Backup -s mein-laptop -t /tmp/restore-test -n
# Echt ins Staging
backuptool restore -S /Volumes/Backup -s mein-laptop -t /tmp/restore-test
# Ins laufende System (uid/gid brauchen root)
sudo backuptool restore -S /Volumes/Backup -s mein-laptop -t /
```
- `-S` Backup-Ordner, `-s` Set, `-t` Ziel-Wurzel (Standard `/`).
- `--no-meta` überspringt das Wiederherstellen von Rechten/Eigentümer.
- Verschlüsseltes Set: `restore` erkennt es am Manifest und fragt nach dem Passwort
  (oder liest `BACKUPTOOL_PASSWORD`).
- Rechte, Eigentümer (als root) und mtime kommen aus dem Manifest — das macht das
  Wiederherstellen von einer **exFAT**-Platte verlustfrei.

> ⚠️ `-t /` verändert das laufende System. Mit `-n` testen oder zuerst in ein
> Staging-Verzeichnis zurückspielen.

---

## 7. exFAT / plattformübergreifende Platten

exFAT ist auf Linux, macOS und Windows les-/schreibbar, kann aber keine Unix-Rechte
speichern. Hier egal: Die Metadaten stehen im Manifest und werden beim Restore wieder
gesetzt. Symlinks (die exFAT nicht ablegen kann) stehen im Manifest und werden beim
Restore auf einem fähigen Dateisystem neu erzeugt. So ist eine exFAT-Platte ideal für
ein überall nutzbares Backup.

---

## 8. GUI

Start: Menüeintrag *backuptool*, `backuptool gui` (Python), `backuptool-gui` (Rust),
oder `./backuptool-portable` ohne Argumente (Python).

- **Sprachauswahl** (oben rechts): English / Deutsch / فارسی. Neue Sprache durch
  Ablegen einer `lang/<code>.json`-Datei.
- **Backup-Reiter:** Quellen hinzufügen, Ziel wählen, Set-Name und Worker-Zahl,
  *Prüfsummen-Abgleich* / *Gelöschtes spiegeln* / *Systemdateien einbeziehen*, und
  (Rust) die **Verschlüsselung** per Dropdown samt Passwort. *Backup starten*;
  Fortschritt + Log unten.
- **Restore-Reiter** (Rust): Backup-Ordner wählen, *Sets laden*, Set wählen,
  Ziel-Wurzel, optional *Probelauf*, Passwort für verschlüsselte Sets, *Restore
  starten*.

---

## 9. Komplette CLI-Referenz

```
backup  QUELLEN...  -d ZIEL  [-s SET] [-c] [-j N] [-e MUSTER ...] [--delete]
                    [--system|--no-system] [-n]
                    [--cipher none|aes256gcm|chacha20poly1305]   (nur Rust)
restore -S QUELLE   [-s SET] [-t ZIEL] [-j N] [--no-meta] [-n]
list    -d ZIEL
```

| Schalter | Bedeutung |
|---|---|
| `-d` | Ziel / Sicherungsplatte (backup, list) |
| `-s` | Backup-Set-Name (Standard: Hostname) |
| `-c` | Prüfsummen-Abgleich (SHA-256 Python / BLAKE3 Rust) |
| `-j N` | parallele Worker (Standard: CPU-Kerne) |
| `-e` | Ausschlussmuster, mehrfach (Syntax abweichend — §3) |
| `--delete` | Gelöschtes im Ziel spiegeln |
| `--system` / `--no-system` | System-Dirs (/etc, …); auto bei root |
| `--cipher` | Verschlüsselung (nur Rust) |
| `-n` | Probelauf |
| `-S` | Backup-Ordner (restore) |
| `-t` | Ziel-Wurzel für Restore (Standard `/`) |
| `--no-meta` | Rechte/Eigentümer beim Restore nicht anwenden |

Siehe auch: [BUILDING.de.md](BUILDING.de.md) und [INSTALL.de.md](INSTALL.de.md).
