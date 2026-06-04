# Usage Guide

How to take backups in every supported way — choosing paths, excluding
directories, with or without encryption, incremental compares, and restoring.
Applies to both implementations; **differences between Python and Rust are
flagged** where they matter.

- Python variant: `backuptool` (or `./backuptool-portable`) — checksum compare uses
  **SHA-256**, **no encryption**.
- Rust variant: `backuptool` — checksum compare uses **BLAKE3**, supports
  **encryption** (`--cipher`).

> In the examples, `backuptool` is the command. With the portable Python launcher
> use `./backuptool-portable` instead; with a freshly built Rust binary use
> `./target/release/backuptool`.

---

## 1. Core concepts

- **Backup set** — a named copy of one system inside the destination. Pick the name
  with `-s NAME` (default: the hostname). Several systems can share one drive:
  ```
  /Volumes/Backup/
    my-laptop/      .backuptool-manifest.json + files
    work-pc/        .backuptool-manifest.json + files
  ```
- **Manifest** — `<<set>>/.backuptool-manifest.json` records every file's path, size,
  mtime, mode, uid/gid (and, in checksum mode, its hash). It drives the incremental
  compare and lets a restore reapply permissions even from exFAT.
- **Incremental** — only changed/new files are copied. A file counts as changed when
  **size or mtime** differ (default) or when the **checksum** differs (`-c`).

---

## 2. Choosing paths

### Sources (what to back up)
One or more files/folders, given as positional arguments:
```bash
backuptool backup ~/Documents ~/Pictures ~/.config -d /Volumes/Backup
```
Paths are stored **absolute** under the set, so `/home/me/Documents/a.txt` becomes
`<<dest>>/<<set>>/home/me/Documents/a.txt`. (On macOS, symlinked roots like `/tmp`
resolve to their real path, e.g. `/private/tmp`.)

### Destination (`-d`)
The backup drive or folder. Created if missing:
```bash
backuptool backup ~/ -d /Volumes/Backup        # external drive
backuptool backup ~/ -d /mnt/nas/backups       # network mount
```

### Set name (`-s`)
```bash
backuptool backup ~/ -d /Volumes/Backup -s my-laptop
backuptool list -d /Volumes/Backup             # see all sets on the drive
```

---

## 3. Excluding directories / files

Pass `-e PATTERN` (repeatable). Caches, trash and thumbnails are excluded by
default. **The pattern syntax differs between the two variants:**

| | Python | Rust |
|---|---|---|
| Engine | `fnmatch` on the full path | `globset` (`**` crosses dirs) |
| Exclude a folder anywhere | `-e '*/node_modules/*'` | `-e '**/node_modules/**'` |
| Exclude by extension | `-e '*.iso'` | `-e '**/*.iso'` |
| Exclude one path | `-e '/home/me/big/*'` | `-e '/home/me/big/**'` |

Examples:
```bash
# Python
backuptool backup ~/ -d /Volumes/Backup -e '*/node_modules/*' -e '*/.git/*' -e '*.iso'

# Rust
backuptool backup ~/ -d /Volumes/Backup -e '**/node_modules/**' -e '**/.git/**' -e '**/*.iso'
```
Default excludes (both): `.cache`, Trash, `.thumbnails`, browser caches,
`lost+found`, and macOS cruft (`.Spotlight-V100` / `.Trashes` / `.fseventsd`; the
Rust variant also skips `.DS_Store`).

---

## 4. Incremental compare modes

```bash
# Fast (default): changed if size or mtime differ
backuptool backup ~/ -d /Volumes/Backup

# Thorough: changed if the content hash differs (uses all CPU cores)
backuptool backup ~/ -d /Volumes/Backup -c        # Python: SHA-256 | Rust: BLAKE3

# Mirror: also delete from the destination what was removed from the source
backuptool backup ~/ -d /Volumes/Backup --delete

# Workers (parallelism), default = CPU cores
backuptool backup ~/ -d /Volumes/Backup -j 8

# Dry run: show what would happen, write nothing
backuptool backup ~/ -d /Volumes/Backup -n
```

---

## 5. Encryption — *Rust variant only*

Choose the cipher with `--cipher`. The content of each file is stored encrypted; the
manifest stays readable (paths/metadata only). The key is derived from a password
with Argon2id (random salt per set).

```bash
# No encryption (default)
backuptool backup ~/ -d /Volumes/Backup

# AES-256-GCM (hardware-accelerated on modern CPUs) — prompts for a password
backuptool backup ~/ -d /Volumes/Backup --cipher aes256gcm

# ChaCha20-Poly1305 (fast without AES hardware)
backuptool backup ~/ -d /Volumes/Backup --cipher chacha20poly1305
```

Non-interactive (automation / cron) — supply the password via an environment
variable instead of the prompt:
```bash
BACKUPTOOL_PASSWORD='your-secret' backuptool backup ~/ -d /Volumes/Backup --cipher aes256gcm
```

> Keep the password safe — **without it the encrypted backup cannot be restored.**
> The cipher and salt are recorded in the manifest, so `restore` knows to ask for
> the password automatically.

The Python variant does not encrypt. For encrypted backups, use the Rust binary
(its manifest format is otherwise the same).

---

## 6. Restore

```bash
# Always dry-run first into a staging dir (writes nothing)
backuptool restore -S /Volumes/Backup -s my-laptop -t /tmp/restore-test -n

# Restore into the staging dir for real
backuptool restore -S /Volumes/Backup -s my-laptop -t /tmp/restore-test

# Restore into the live system (uid/gid require root)
sudo backuptool restore -S /Volumes/Backup -s my-laptop -t /
```

- `-S` backup folder, `-s` set, `-t` target root (default `/`).
- `--no-meta` skips reapplying permissions/owner.
- Encrypted set: `restore` detects it from the manifest and asks for the password
  (or reads `BACKUPTOOL_PASSWORD`).
- Permissions, owner (as root) and mtime are reapplied from the manifest — this is
  what makes restoring from an **exFAT** drive lossless.

> ⚠️ `-t /` modifies the running system. Test with `-n` or restore into a staging
> directory first, then copy what you need.

---

## 7. exFAT / cross-platform drives

exFAT is readable/writable on Linux, macOS and Windows but cannot store Unix
permissions. That is fine here: the metadata lives in the manifest and is reapplied
on restore. Symlinks (which exFAT cannot store) are recorded in the manifest and
recreated on restore on a capable filesystem. So an exFAT drive is the best choice
for a backup you want to use everywhere.

---

## 8. Using the GUI

Launch: menu entry *backuptool*, `backuptool gui` (Python), `backuptool-gui` (Rust),
or `./backuptool-portable` with no arguments (Python).

- **Language selector** (top right): English / Deutsch / فارسی. Add a language by
  dropping a `lang/<<code>>.json` file next to the others.
- **Backup tab:** add sources, choose the destination, set the name and worker
  count, tick *Checksum compare* / *Mirror deletions*, and (Rust) pick the
  **encryption** from the dropdown and enter a password. Press *Start backup*;
  progress and a log appear at the bottom.
- **Restore tab** (Rust): choose the backup folder, click *Load sets*, pick the set,
  set the target root, optionally tick *Dry run*, enter the password for encrypted
  sets, and press *Start restore*.

---

## 9. Full CLI reference

```
backup  SOURCES...  -d DEST  [-s SET] [-c] [-j N] [-e PATTERN ...] [--delete] [-n]
                    [--cipher none|aes256gcm|chacha20poly1305]   (Rust only)
restore -S SOURCE   [-s SET] [-t TARGET] [-j N] [--no-meta] [-n]
list    -d DEST
```

| Flag | Meaning |
|---|---|
| `-d` | destination / backup drive (backup, list) |
| `-s` | backup set name (default: hostname) |
| `-c` | checksum compare (SHA-256 Python / BLAKE3 Rust) |
| `-j N` | parallel workers (default: CPU cores) |
| `-e` | exclude pattern, repeatable (syntax differs — see §3) |
| `--delete` | mirror deletions in the destination |
| `--cipher` | encryption (Rust only) |
| `-n` | dry run |
| `-S` | backup folder (restore) |
| `-t` | target root for restore (default `/`) |
| `--no-meta` | do not reapply permissions/owner on restore |

See also: [BUILDING.md](BUILDING.md) and [INSTALL.md](INSTALL.md).
