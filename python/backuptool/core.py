# SPDX-License-Identifier: GPL-3.0-or-later
"""Core engine: scan, incremental diff, parallel copy, manifest, restore.

Cross-platform (Linux/macOS), standard library only.

File metadata (permissions, owner, mtime) is stored in the manifest. This lets
them be restored even when the destination filesystem cannot store them
(e.g. exFAT) – solving the exFAT problem without tar.
"""
from __future__ import annotations

import os
import sys
import json
import time
import stat
import base64
import shutil
import hashlib
import fnmatch
import socket
from concurrent.futures import ThreadPoolExecutor, ProcessPoolExecutor, as_completed

from . import crypto
from . import reset

MANIFEST_NAME = ".backuptool-manifest.json"
HISTORY_DIR = ".backuptool-history"

# File kinds. Kept as the JSON strings "file"/"symlink" so the manifest stays
# compatible with the Rust implementation.
KIND_FILE = "file"
KIND_SYMLINK = "symlink"

# Default excludes (cache / trash / temporary)
DEFAULT_EXCLUDES = [
    "*/.cache/*", "*/.cache",
    "*/.local/share/Trash/*", "*/.local/share/Trash",
    "*/.thumbnails/*", "*/.thumbnails",
    "*/Cache/*", "*/Cache",
    "*/cache2/*",
    "*/.gvfs", "*.sock", "lost+found",
    "*/.Spotlight-V100/*", "*/.Trashes/*", "*/.fseventsd/*",
    # Volatile / always-locked junk (mainly Windows): browser caches, temp, and
    # the unreadable WindowsApps app-execution aliases.
    "*/Code Cache/*", "*/Code Cache", "*/GPUCache/*", "*/GPUCache",
    "*/DawnGraphiteCache/*", "*/DawnWebGPUCache/*",
    "*/AppData/Local/Temp/*", "*/AppData/Local/Temp",
    "*/AppData/Local/Microsoft/WindowsApps/*", "*/AppData/Local/Microsoft/WindowsApps",
]


# ----------------------------------------------------------------------------
# Helpers
# ----------------------------------------------------------------------------
def default_workers() -> int:
    return max(1, (os.cpu_count() or 2))


# Cap on individual error lines logged per run (rest are summarized) so a failing
# source can't produce a huge log or flood the GUI thread.
ERROR_LOG_CAP = 200


def _to_rel(full: str) -> str:
    """Destination-relative path: strip leading '/' (unix) or the drive / \\\\?\\
    prefix (Windows) so files land UNDER the destination instead of escaping via
    an absolute join. A drive like 'D:\\' becomes a top-level folder 'D'."""
    s = full
    if s.startswith("\\\\?\\"):
        s = s[4:]
    if len(s) >= 2 and s[1] == ":" and s[0].isalpha():
        s = s[0] + s[2:]          # 'C:\Users' -> 'C\Users'
    return s.lstrip("/\\")


def _is_volume_root(s: str) -> bool:
    """True if s names a whole volume/drive root (/, C:, C:\\, \\\\?\\C:\\)."""
    t = (s or "").strip()
    if t == "/":
        return True
    if t.startswith("\\\\?\\"):
        t = t[4:]
    return (len(t) == 2 and t[1] == ":") or (len(t) == 3 and t[1] == ":" and t[2] in "\\/")


def system_dirs() -> list:
    """Curated system directories worth backing up (e.g. /etc). Returns the ones
    that exist and are readable — best run as root to read them fully."""
    if sys.platform == "darwin":
        cands = ["/etc", "/usr/local/etc", "/opt"]
    elif sys.platform.startswith("linux"):
        cands = ["/etc", "/usr/local/etc", "/opt", "/srv", "/root", "/var/spool/cron"]
    else:
        cands = []
    return [d for d in cands if os.path.isdir(d) and os.access(d, os.R_OK)]


def _mount_root(path: str) -> str:
    """The volume/mount root that `path` lives on (Windows: the drive root)."""
    path = os.path.abspath(path)
    if os.name == "nt":
        drive = os.path.splitdrive(path)[0]
        return (drive + os.sep) if drive else path
    p = path
    while not os.path.ismount(p):
        parent = os.path.dirname(p)
        if parent == p:
            break
        p = parent
    return p


def suggested_dest() -> str:
    """Suggested backup destination: the volume the running app lives on, plus a
    dated, per-user folder (e.g. /Volumes/Stick/backuptool-20260613-alice).
    Handy for portable use — run from the backup drive and it targets that drive."""
    import getpass
    try:
        base = os.path.dirname(os.path.abspath(__file__))
    except NameError:
        base = os.getcwd()
    vol = _mount_root(base)
    try:
        user = getpass.getuser() or "user"
    except Exception:
        user = "user"
    name = "backuptool-" + time.strftime("%Y%m%d") + "-" + user
    return os.path.join(vol, name)


def human(n: float) -> str:
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if n < 1024:
            return f"{n:.1f} {unit}"
        n /= 1024
    return f"{n:.1f} PB"


def sha256_file(path: str, _bufsize: int = 1 << 20):
    """SHA-256 of a file (runs in a ProcessPool – uses multiple CPU cores)."""
    h = hashlib.sha256()
    try:
        with open(path, "rb", buffering=0) as f:
            for chunk in iter(lambda: f.read(_bufsize), b""):
                h.update(chunk)
    except OSError:
        return None
    return h.hexdigest()


def is_excluded(path: str, patterns) -> bool:
    return any(fnmatch.fnmatch(path, p) for p in patterns)


def _is_subpath(child: str, parent: str) -> bool:
    parent = parent.rstrip("/") or "/"
    return child == parent or child.startswith(parent + "/")


def analyze_overlaps(paths) -> list:
    """Find sources already covered by another selected source.

    Returns a list of {"path": original, "covered_by": original} for every entry
    that is a sub-path of (or a duplicate of an earlier) entry. The copy step
    de-duplicates anyway; this lets the UI warn before doing redundant work."""
    norm = [os.path.realpath(os.path.abspath(p)) for p in paths]
    out = []
    for i, p in enumerate(norm):
        for j, q in enumerate(norm):
            if i == j:
                continue
            if (p == q and j < i) or (p != q and _is_subpath(p, q)):
                out.append({"path": paths[i], "covered_by": paths[j]})
                break
    return out


def brief_listing(path: str, limit: int = 40) -> list:
    """A short, sorted sample of a directory's entries (for the overlap prompt)."""
    try:
        names = sorted(os.listdir(path))
    except OSError:
        return []
    return names[:limit]


# ----------------------------------------------------------------------------
# Scan
# ----------------------------------------------------------------------------
def _add_entry(entries: dict, full: str, excludes) -> None:
    if is_excluded(full, excludes):
        return
    try:
        st = os.lstat(full)
    except OSError:
        return
    mode = st.st_mode
    meta = {
        "size": st.st_size,
        # integer seconds — keeps the manifest type-compatible with the Rust tool
        # (which stores mtime as i64), so backups are interchangeable.
        "mtime": int(st.st_mtime),
        "mode": mode,
        "uid": st.st_uid,
        "gid": st.st_gid,
    }
    if stat.S_ISLNK(mode):
        meta["type"] = KIND_SYMLINK
        try:
            meta["target"] = os.readlink(full)
        except OSError:
            meta["target"] = ""
    elif stat.S_ISREG(mode):
        meta["type"] = KIND_FILE
    else:
        return  # skip sockets/FIFOs/devices
    rel = _to_rel(full)
    meta["path"] = full
    entries[rel] = meta


def scan(sources, excludes) -> dict:
    """Return {relative_path: meta} for every file to back up."""
    entries: dict = {}
    for src in sources:
        src = os.path.abspath(src)
        if os.path.islink(src) or os.path.isfile(src):
            _add_entry(entries, src, excludes)
            continue
        for root, dirs, files in os.walk(src, followlinks=False):
            dirs[:] = [d for d in dirs
                       if not is_excluded(os.path.join(root, d), excludes)]
            for name in files:
                _add_entry(entries, os.path.join(root, name), excludes)
    return entries


# ----------------------------------------------------------------------------
# Manifest
# ----------------------------------------------------------------------------
def load_manifest(path: str):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def save_manifest(manifest_path: str, man: dict, set_root: str) -> None:
    os.makedirs(set_root, exist_ok=True)
    tmp = manifest_path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(man, f, indent=1)
    os.replace(tmp, manifest_path)
    # history (one copy per run)
    hist = os.path.join(set_root, HISTORY_DIR)
    try:
        os.makedirs(hist, exist_ok=True)
        stamp = man["created"].replace(":", "").replace("-", "").replace("T", "-")
        shutil.copy2(manifest_path, os.path.join(hist, f"manifest-{stamp}.json"))
    except OSError:
        pass


def list_sets(backup_dir: str):
    """All backup sets (subfolders with a manifest) inside a destination."""
    out = []
    try:
        for name in sorted(os.listdir(backup_dir)):
            mp = os.path.join(backup_dir, name, MANIFEST_NAME)
            man = load_manifest(mp)
            if man:
                out.append({
                    "set": name,
                    "host": man.get("host", "?"),
                    "created": man.get("created", "?"),
                    "files": len(man.get("files", {})),
                })
    except OSError:
        pass
    return out


# ----------------------------------------------------------------------------
# Incremental diff
# ----------------------------------------------------------------------------
def needs_copy(meta: dict, prev: dict, use_checksum: bool) -> bool:
    if prev is None:
        return True
    if meta.get("type") != prev.get("type"):
        return True
    if meta.get("type") == KIND_SYMLINK:
        return meta.get("target") != prev.get("target")
    if use_checksum:
        return meta.get("sha256") != prev.get("sha256")
    return (int(meta["size"]) != int(prev.get("size", -1)) or
            int(meta["mtime"]) != int(prev.get("mtime", -1)))


# ----------------------------------------------------------------------------
# Copy
# ----------------------------------------------------------------------------
def copy_one(meta: dict, set_root: str, rel: str, cipher=crypto.CIPHER_NONE, key=None):
    dst = os.path.join(set_root, rel)
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    if meta["type"] == KIND_SYMLINK:
        try:
            if os.path.lexists(dst):
                os.remove(dst)
            os.symlink(meta.get("target", ""), dst)
        except OSError:
            pass  # destination FS without symlinks (exFAT): info stays in manifest
        return 0
    if cipher == crypto.CIPHER_NONE:
        shutil.copy2(meta["path"], dst, follow_symlinks=False)
    else:
        with open(meta["path"], "rb") as f:
            data = f.read()
        with open(dst, "wb") as f:
            f.write(crypto.encrypt(cipher, key, data))
    return meta["size"]


# ----------------------------------------------------------------------------
# Backup
# ----------------------------------------------------------------------------
def backup(sources, dest, setname=None, workers=None, use_checksum=False,
           extra_excludes=None, prune=False, dry_run=False,
           include_system=False, cipher=crypto.CIPHER_NONE, passphrase=None,
           vss_map=None, verify=True, log=print, progress=None):
    workers = workers or default_workers()
    cipher = crypto.normalize_cipher(cipher)
    excludes = list(extra_excludes or []) + DEFAULT_EXCLUDES
    setname = setname or socket.gethostname()
    set_root = os.path.join(os.path.abspath(dest), setname)
    manifest_path = os.path.join(set_root, MANIFEST_NAME)

    sources = list(sources)
    # Selecting a whole volume root (C:\, /) auto-excludes OS/program dirs so
    # "back up C:" copies user data without drowning in Windows system files.
    if not include_system and any(_is_volume_root(s) for s in sources):
        excludes += disk_excludes()
        log("Volume root selected — auto-excluding system/OS directories (Windows, Program Files, …).")
    if include_system:
        sysdirs = system_dirs()
        log(f"Including system directories: {', '.join(sysdirs) or '(none)'}")
        sources += sysdirs

    prev = load_manifest(manifest_path)
    prev_files = prev.get("files", {}) if prev else {}

    # Derive the key, reusing the previous set's salt/params when the cipher is
    # unchanged (so incremental runs stay decryptable with the same password).
    kdf = None
    key = None
    if cipher != crypto.CIPHER_NONE:
        if not passphrase:
            raise ValueError("password missing for encryption")
        if (prev and prev.get("cipher") == cipher and prev.get("kdf")):
            pk = prev["kdf"]
            salt = base64.b64decode(pk["salt_b64"])
            m, t, p = pk["m_cost"], pk["t_cost"], pk["p_cost"]
        else:
            salt = crypto.random_salt()
            m, t, p = crypto.DEFAULT_M_COST, crypto.DEFAULT_T_COST, crypto.DEFAULT_P_COST
        key = crypto.derive_key(passphrase, salt, m, t, p)
        kdf = {"algo": "argon2id",
               "salt_b64": base64.b64encode(salt).decode("ascii"),
               "m_cost": m, "t_cost": t, "p_cost": p}

    log("Scanning sources ...")
    entries = scan(sources, excludes)
    # VSS: read from the shadow copy but record files under their original path.
    if vss_map:
        remapped = {}
        for rel, m in entries.items():
            p = m.get("path", "")
            newrel = rel
            for shadow, orig in vss_map:
                if p.startswith(shadow):
                    newrel = _to_rel(orig + p[len(shadow):])
                    break
            remapped[newrel] = m
        entries = remapped
    log(f"{len(entries)} files found.")

    if use_checksum and entries:
        log(f"Computing SHA-256 with {workers} processes (multicore) ...")
        rels = list(entries.keys())
        paths = [entries[r]["path"] for r in rels]
        try:
            with ProcessPoolExecutor(max_workers=workers) as ex:
                for rel, digest in zip(rels, ex.map(sha256_file, paths, chunksize=8)):
                    entries[rel]["sha256"] = digest
        except Exception as e:  # fall back to single core
            log(f"  ProcessPool unavailable ({e}) – computing serially.")
            for rel in rels:
                entries[rel]["sha256"] = sha256_file(entries[rel]["path"])

    todo = [r for r, m in entries.items()
            if needs_copy(m, prev_files.get(r), use_checksum)]
    unchanged = len(entries) - len(todo)
    total_bytes = sum(entries[r]["size"] for r in todo if entries[r]["type"] == KIND_FILE)
    deletions = [r for r in prev_files if r not in entries]
    log(f"Changed/new: {len(todo)} ({human(total_bytes)}) | unchanged: {unchanged} "
        f"| deleted in source: {len(deletions)}")

    if dry_run:
        for r in todo[:1000]:
            log(f"  [would back up] /{r}")
        if deletions:
            log(f"  {len(deletions)} deleted file(s) would be "
                f"{'removed' if prune else 'kept'}.")
        return {"copied": 0, "skipped": unchanged, "bytes": 0, "dryrun": True}

    os.makedirs(set_root, exist_ok=True)
    done = copied = errors = 0
    copied_bytes = 0
    err_cats = {"in_use": 0, "denied": 0, "not_found": 0, "other": 0}
    total = len(todo)
    with ThreadPoolExecutor(max_workers=workers) as ex:
        futs = {ex.submit(copy_one, entries[r], set_root, r, cipher, key): r for r in todo}
        for fut in as_completed(futs):
            r = futs[fut]
            done += 1
            try:
                nbytes = fut.result()
                copied += 1
                copied_bytes += nbytes
            except Exception as e:
                errors += 1
                win = getattr(e, "winerror", None)
                msg = str(e)
                if win == 32 or "being used by another process" in msg:
                    err_cats["in_use"] += 1
                elif win in (5, 13) or isinstance(e, PermissionError) or "Access is denied" in msg:
                    err_cats["denied"] += 1
                elif win in (2, 3) or isinstance(e, FileNotFoundError):
                    err_cats["not_found"] += 1
                else:
                    err_cats["other"] += 1
                if errors <= ERROR_LOG_CAP:
                    log(f"  ERROR {r}: {e}")
                elif errors == ERROR_LOG_CAP + 1:
                    log(f"  ... further errors suppressed (over {ERROR_LOG_CAP}); see the summary below.")
            if progress:
                progress(done, total, r)

    deleted = 0
    if prune:
        for r in deletions:
            tgt = os.path.join(set_root, r)
            try:
                if os.path.lexists(tgt):
                    os.remove(tgt)
                    deleted += 1
            except OSError:
                pass

    man = {
        "version": 1,
        "set": setname,
        "host": socket.gethostname(),
        "platform": sys.platform,
        "created": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "sources": [os.path.abspath(s) for s in sources],
        "use_checksum": use_checksum,
        "cipher": cipher,
        "files": {r: {k: v for k, v in m.items() if k != "path"}
                  for r, m in entries.items()},
    }
    if kdf is not None:
        man["kdf"] = kdf
    save_manifest(manifest_path, man, set_root)

    # Per-run, dated log listing exactly which full paths changed/were removed.
    logpath = None
    try:
        logdir = os.path.join(set_root, ".backuptool-logs")
        os.makedirs(logdir, exist_ok=True)
        stamp = man["created"].replace(":", "").replace("-", "").replace("T", "-")
        logpath = os.path.join(logdir, f"backup-{stamp}.log")
        with open(logpath, "w", encoding="utf-8") as lf:
            lf.write(f"# backuptool  set={setname}  host={man['host']}  {man['created']}\n")
            lf.write(f"# sources: {', '.join(man['sources'])}\n")
            lf.write(f"# changed/new={copied} unchanged={unchanged} deleted={deleted} "
                     f"errors={errors} bytes={copied_bytes}\n")
            for r in sorted(todo):
                lf.write(f"CHANGED\t/{r}\n")
            for r in sorted(deletions):
                lf.write(f"DELETED\t/{r}\n")
        log(f"Log written: {logpath}")
    except OSError:
        pass

    if errors:
        log(f"Errors by type: in-use(locked)={err_cats['in_use']}, "
            f"access-denied={err_cats['denied']}, not-found={err_cats['not_found']}, "
            f"other={err_cats['other']}.")
        log("Tip: 'in-use' files were open in another app — close it (or use clone/VSS) and re-run.")
    log(f"Done: {copied} copied ({human(copied_bytes)}), {unchanged} skipped, "
        f"{deleted} removed, {errors} errors.")

    # Control: loud warning if a non-empty selection produced nothing usable.
    if entries and copied == 0 and unchanged == 0:
        log("WARNING: 0 files reached the destination! Nothing was backed up — "
            "check the errors above and the destination path.")

    # Control: read the destination back and confirm the files are really there.
    verified = None
    if verify:
        log("Verifying destination (reading files back) ...")
        try:
            v = verify_set(set_root, workers=workers, passphrase=passphrase, log=log)
            verified = v
            if v["errors"] > 0:
                log(f"WARNING: verification found {v['errors']} file(s) MISSING or "
                    f"corrupt at the destination!")
            else:
                log(f"Verified OK: {v['ok']} file(s) confirmed at the destination.")
        except Exception as e:  # noqa: BLE001
            log(f"Verification could not run: {e}")

    return {"copied": copied, "skipped": unchanged, "bytes": copied_bytes,
            "deleted": deleted, "errors": errors, "log": logpath,
            "verified": verified}


# ----------------------------------------------------------------------------
# Restore
# ----------------------------------------------------------------------------
def _am_root() -> bool:
    return hasattr(os, "geteuid") and os.geteuid() == 0


def _derive_key_for(man: dict, cipher: str, passphrase):
    """Derive the decryption key from a manifest's KDF parameters, or None when
    the set is not encrypted."""
    if cipher == crypto.CIPHER_NONE:
        return None
    kdf = man.get("kdf")
    if not kdf:
        raise ValueError("KDF parameters missing in manifest")
    if not passphrase:
        raise ValueError("password missing")
    salt = base64.b64decode(kdf["salt_b64"])
    return crypto.derive_key(passphrase, salt, kdf["m_cost"], kdf["t_cost"], kdf["p_cost"])


def restore(backup_dir, setname=None, target="/", workers=None,
            reapply_meta=True, dry_run=False, passphrase=None,
            log=print, progress=None):
    if setname:
        set_root = os.path.join(os.path.abspath(backup_dir), setname)
    else:
        set_root = os.path.abspath(backup_dir)
    manifest_path = os.path.join(set_root, MANIFEST_NAME)
    man = load_manifest(manifest_path)
    if not man:
        raise FileNotFoundError(f"No manifest at {manifest_path}")

    cipher = crypto.normalize_cipher(man.get("cipher", "none"))
    key = _derive_key_for(man, cipher, passphrase)

    files = man.get("files", {})
    target = os.path.abspath(target)
    workers = workers or default_workers()
    items = list(files.items())
    total = len(items)
    am_root = _am_root()
    log(f"Restoring {total} entries -> {target}")

    if dry_run:
        for rel, _ in items[:1000]:
            log(f"  [would restore] {os.path.join(target, rel)}")
        return {"restored": 0, "dryrun": True}

    def _restore_one(rel, meta):
        src = os.path.join(set_root, rel)
        dst = os.path.join(target, rel)
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        if meta.get("type") == KIND_SYMLINK:
            if os.path.lexists(dst):
                os.remove(dst)
            os.symlink(meta.get("target", ""), dst)
            return 0
        if not os.path.exists(src):
            raise FileNotFoundError(src)
        if cipher == crypto.CIPHER_NONE:
            shutil.copy2(src, dst, follow_symlinks=False)
        else:
            with open(src, "rb") as f:
                blob = f.read()
            with open(dst, "wb") as f:
                f.write(crypto.decrypt(cipher, key, blob))
        if reapply_meta:
            try:
                os.chmod(dst, stat.S_IMODE(meta["mode"]))
            except OSError:
                pass
            if am_root:
                try:
                    os.chown(dst, meta["uid"], meta["gid"])
                except OSError:
                    pass
            try:
                os.utime(dst, (meta["mtime"], meta["mtime"]))
            except OSError:
                pass
        return meta.get("size", 0)

    done = restored = errors = 0
    with ThreadPoolExecutor(max_workers=workers) as ex:
        futs = {ex.submit(_restore_one, rel, meta): rel for rel, meta in items}
        for fut in as_completed(futs):
            rel = futs[fut]
            done += 1
            try:
                fut.result()
                restored += 1
            except Exception as e:
                errors += 1
                log(f"  ERROR {rel}: {e}")
            if progress:
                progress(done, total, rel)

    log(f"Restore done: {restored} restored, {errors} errors.")
    if reapply_meta and not am_root:
        log("Note: without root, owner (uid/gid) could not be set – "
            "re-run with sudo if needed.")
    return {"restored": restored, "errors": errors}


# ----------------------------------------------------------------------------
# Verify
# ----------------------------------------------------------------------------
def verify_set(set_root, workers=None, passphrase=None, log=print, progress=None):
    """Re-read every file of a backup set from the destination (decrypting it
    when the set is encrypted) and compare its SHA-256 (or size, when no hash was
    recorded) against the manifest. Returns {"ok": n, "errors": n}."""
    workers = workers or default_workers()
    manifest_path = os.path.join(os.path.abspath(set_root), MANIFEST_NAME)
    man = load_manifest(manifest_path)
    if not man:
        raise FileNotFoundError(f"No manifest at {manifest_path}")

    cipher = crypto.normalize_cipher(man.get("cipher", "none"))
    key = _derive_key_for(man, cipher, passphrase)
    files = man.get("files", {})
    items = list(files.items())
    total = len(items)
    log(f"Verifying {total} entries ...")

    def _plaintext(src):
        with open(src, "rb") as f:
            blob = f.read()
        return crypto.decrypt(cipher, key, blob) if cipher != crypto.CIPHER_NONE else blob

    def _verify_one(rel, meta):
        src = os.path.join(set_root, rel)
        if meta.get("type") == KIND_SYMLINK:
            got = os.readlink(src)  # raises OSError if missing
            if got != meta.get("target", ""):
                raise ValueError("symlink target mismatch")
            return True
        recorded = meta.get("sha256")
        if cipher == crypto.CIPHER_NONE and recorded is not None:
            if sha256_file(src) != recorded:
                raise ValueError("SHA-256 mismatch")
        elif recorded is not None:
            if hashlib.sha256(_plaintext(src)).hexdigest() != recorded:
                raise ValueError("SHA-256 mismatch")
        else:
            if len(_plaintext(src)) != int(meta.get("size", -1)):
                raise ValueError("size mismatch")
        return True

    done = ok = errors = 0
    with ThreadPoolExecutor(max_workers=workers) as ex:
        futs = {ex.submit(_verify_one, rel, meta): rel for rel, meta in items}
        for fut in as_completed(futs):
            rel = futs[fut]
            done += 1
            try:
                fut.result()
                ok += 1
            except Exception as e:
                errors += 1
                log(f"  VERIFY-FAIL {rel}: {e}")
            if progress:
                progress(done, total, rel)

    log(f"Verify done: {ok} ok, {errors} failed.")
    return {"ok": ok, "errors": errors}


# ----------------------------------------------------------------------------
# Evacuate (decommission)
# ----------------------------------------------------------------------------
def home_roots() -> list:
    """User-data root(s) for the current platform."""
    if sys.platform == "darwin":
        return ["/Users"]
    if sys.platform.startswith("linux"):
        return ["/home"]
    if sys.platform.startswith("win"):
        drive = os.environ.get("SystemDrive", "C:")
        return [os.path.join(drive + os.sep, "Users")]
    return []


def disk_root() -> str:
    if sys.platform.startswith("win"):
        return os.environ.get("SystemDrive", "C:") + os.sep
    return "/"


def disk_excludes() -> list:
    """Glob patterns protecting OS/system locations when evacuating a whole disk."""
    if sys.platform == "darwin":
        return ["/System/*", "/usr/*", "/bin/*", "/sbin/*", "/private/*", "/Library/*",
                "/Applications/*", "/Volumes/*", "/dev/*", "/cores/*", "/.vol/*",
                "/.fseventsd/*"]
    if sys.platform.startswith("linux"):
        return ["/usr/*", "/bin/*", "/sbin/*", "/lib/*", "/lib64/*", "/boot/*",
                "/proc/*", "/sys/*", "/dev/*", "/run/*", "/var/*", "/etc/*", "/tmp/*",
                "/snap/*", "/mnt/*", "/media/*", "lost+found"]
    if sys.platform.startswith("win"):
        # Exclude the OS and program directories; keep user data (C:\Users, etc.).
        # fnmatch on Windows is case-insensitive and treats / and \ alike, so a
        # '*/Name' pattern prunes the folder and '*/Name/*' its contents.
        names = ["Windows", "Program Files", "Program Files (x86)", "ProgramData",
                 "$Recycle.Bin", "System Volume Information", "Recovery", "PerfLogs",
                 "$WinREAgent", "Windows.old", "Config.Msi"]
        pats = []
        for n in names:
            pats += [f"*/{n}", f"*/{n}/*"]
        pats += ["*/pagefile.sys", "*/hiberfil.sys", "*/swapfile.sys"]
        return pats
    return []


def scope_sources(scope: str, explicit) -> list:
    """Resolve a scope keyword to concrete source paths.
    'home' -> user data root, 'config' -> the explicit paths, 'disk' -> the whole
    system root (or the explicit mount points)."""
    explicit = list(explicit or [])
    if scope == "config":
        if not explicit:
            raise ValueError("scope=config requires at least one source path")
        return explicit
    if scope == "home":
        roots = home_roots()
        if not roots:
            raise ValueError("could not determine a home directory root on this "
                             "platform; use scope=config")
        return roots
    if scope == "disk":
        return explicit or [disk_root()]
    if scope == "auto":
        a = auto_sources()
        if not a:
            raise ValueError("could not auto-detect user data folders; use scope=config")
        return a
    raise ValueError(f"unknown scope '{scope}' (use: home | config | disk | auto)")


def auto_sources():
    """The current user's canonical data folders for this OS (existing ones only).
    Used by the 'Auto' button / --auto to fill sources without the OS itself."""
    home = os.environ.get("USERPROFILE") or os.path.expanduser("~")
    out = []
    if home and os.path.isdir(home):
        if sys.platform.startswith("win"):
            subs = ["Documents", "Desktop", "Downloads", "Pictures", "Music", "Videos",
                    "Favorites", "Saved Games", "OneDrive", os.path.join("AppData", "Roaming")]
        elif sys.platform == "darwin":
            subs = ["Documents", "Desktop", "Downloads", "Pictures", "Music", "Movies",
                    "Library/Application Support", "Library/Preferences", "Library/Mail"]
        else:
            subs = ["Documents", "Desktop", "Downloads", "Pictures", "Music", "Videos",
                    ".config", ".local/share", ".mozilla", ".thunderbird", ".ssh", ".gnupg"]
        for s in subs:
            p = os.path.join(home, s)
            if os.path.exists(p):
                out.append(p)
        if not out:
            out.append(home)
    return out


def same_device(a: str, b: str):
    """True if both paths are on the same filesystem device, None if unknown."""
    try:
        return os.stat(a).st_dev == os.stat(b).st_dev
    except OSError:
        return None


def free_space(path: str):
    try:
        return shutil.disk_usage(path).free
    except OSError:
        return None


def secure_overwrite(path: str, passes: int = 1) -> None:
    """Overwrite a file's contents with random data (``passes`` times), then
    truncate it to zero length. The caller unlinks it afterwards.
    Note: on SSDs and copy-on-write filesystems (APFS, Btrfs, ZFS) this does NOT
    guarantee the old blocks are unrecoverable — the caller warns about that."""
    length = os.path.getsize(path)
    chunk = 64 * 1024
    with open(path, "r+b", buffering=0) as f:
        for _ in range(max(1, passes)):
            f.seek(0)
            remaining = length
            while remaining > 0:
                n = min(chunk, remaining)
                f.write(os.urandom(n))
                remaining -= n
            f.flush()
            os.fsync(f.fileno())
        f.truncate(0)
        f.flush()
        os.fsync(f.fileno())


def evacuate(sources, dest, setname=None, workers=None, extra_excludes=None,
             scope="config", delete_source=False, allow_same_device=False,
             cipher=crypto.CIPHER_NONE, passphrase=None,
             secure_wipe=False, wipe_passes=1,
             dry_run=False, log=print, progress=None):
    """Evacuate (move) files to external storage: copy + verify, and only after a
    fully verified copy optionally delete the sources. Nothing is ever deleted if
    the backup or verification reports a single error.
    Returns a report dict."""
    workers = workers or default_workers()
    dest = os.path.abspath(dest)
    setname = setname or (socket.gethostname() + "-decommission")
    set_root = os.path.join(dest, setname)

    excludes = list(extra_excludes or [])
    if scope == "disk":
        excludes += disk_excludes()
    scan_excludes = excludes + DEFAULT_EXCLUDES

    os.makedirs(dest, exist_ok=True)

    # Pre-scan to know the source files (for size accounting and deletion).
    entries = scan(sources, scan_excludes)
    scanned = len(entries)
    total_bytes = sum(m["size"] for m in entries.values() if m["type"] == KIND_FILE)
    log(f"{scanned} files to evacuate ({human(total_bytes)}).")

    # Safety: refuse to "move" onto the same physical device as a source.
    if not allow_same_device:
        for s in sources:
            if same_device(s, dest) is True:
                raise RuntimeError(
                    f"Refusing: destination '{dest}' is on the SAME device as source "
                    f"'{s}'. Use an external disk/stick (or allow_same_device=True).")

    # Safety: ensure the destination has room for the data.
    free = free_space(dest)
    if free is not None and free < total_bytes:
        raise RuntimeError(
            f"Not enough free space at '{dest}': need {total_bytes} bytes, have {free}.")

    if dry_run:
        log("[dry-run] would copy + verify the above; no source is deleted.")
        for m in list(entries.values())[:1000]:
            log(f"  [would move] {m['path']}")
        return {"scanned": scanned, "bytes": total_bytes, "copied": 0,
                "verified": 0, "deleted": 0, "delete_errors": 0, "wiped": 0,
                "storage": "", "verify_errors": 0, "backup_errors": 0, "dryrun": True}

    # Copy phase — force checksum so the manifest carries hashes to verify against.
    bstats = backup(sources, dest, setname=setname, workers=workers,
                    use_checksum=True, extra_excludes=excludes, prune=False,
                    dry_run=False, include_system=False, cipher=cipher,
                    passphrase=passphrase, log=log, progress=progress)
    if bstats.get("errors", 0) > 0:
        raise RuntimeError(f"Copy phase had {bstats['errors']} error(s) — "
                           f"aborting, NOTHING deleted.")

    # Verify phase — read everything back and compare hashes.
    log("Verifying copies on destination (SHA-256) ...")
    vstats = verify_set(set_root, workers=workers, passphrase=passphrase,
                        log=log, progress=progress)
    if vstats["errors"] > 0:
        raise RuntimeError(f"Verification failed for {vstats['errors']} file(s) — "
                           f"aborting, NOTHING deleted.")

    report = {"scanned": scanned, "bytes": total_bytes,
              "copied": bstats.get("copied", 0), "backup_errors": 0,
              "verified": vstats["ok"], "verify_errors": 0,
              "deleted": 0, "delete_errors": 0, "wiped": 0, "storage": ""}

    # Delete phase — only reached when copy AND verify are clean.
    if delete_source:
        if secure_wipe:
            log(f"Securely wiping source files ({max(1, wipe_passes)} pass(es)); "
                f"detecting storage per device ...")
        else:
            log("Verification OK — deleting source files ...")
        man = load_manifest(os.path.join(set_root, MANIFEST_NAME))
        confirmed = set((man or {}).get("files", {}).keys())
        deleted = derr = wiped = 0
        parents = set()
        vk_cache = {}        # st_dev -> (partition label, kind); one probe per partition
        part_counts = {}     # partition label -> [kind, file count]
        ssd_reps = {}        # partition label -> a representative path (free-space wipe)
        for rel, m in entries.items():
            if rel not in confirmed:
                continue
            try:
                overwritten = False
                if secure_wipe and m["type"] == KIND_FILE:
                    try:
                        vk = os.stat(m["path"]).st_dev
                    except OSError:
                        vk = None
                    if vk not in vk_cache:
                        dev, kind = reset.storage_info(m["path"])
                        vk_cache[vk] = (dev or str(vk), kind)
                    label, kind = vk_cache[vk]
                    entry = part_counts.setdefault(label, [kind, 0])
                    entry[1] += 1
                    if kind == "ssd":
                        ssd_reps.setdefault(label, m["path"])
                    secure_overwrite(m["path"], wipe_passes)
                    overwritten = True
                os.remove(m["path"])
                deleted += 1
                if overwritten:
                    wiped += 1
                parents.add(os.path.dirname(m["path"]))
            except OSError as e:
                derr += 1
                log(f"  DELETE-FAIL {m['path']}: {e}")
        # Best-effort prune of now-empty directories (deepest first).
        for d in sorted(parents, key=lambda p: p.count(os.sep), reverse=True):
            try:
                os.rmdir(d)
            except OSError:
                pass
        report["deleted"] = deleted
        report["wiped"] = wiped
        report["delete_errors"] = derr
        log(f"Deleted {deleted} source file(s) ({wiped} securely wiped), {derr} error(s).")

        # Act per partition: overwrite is only trustworthy on HDD. With mixed
        # disks we report the mix, advise on the worst case, and (Windows) wipe
        # free space on every distinct SSD partition that held deleted files.
        if secure_wipe and part_counts:
            kinds = {v[0] for v in part_counts.values()}
            report["storage"] = next(iter(kinds)) if len(kinds) == 1 else "mixed"
            log("Storage per partition:")
            for label in sorted(part_counts):
                kind, n = part_counts[label]
                log(f"  {label}  [{kind}]  {n} file(s)")
            if any(k != "hdd" for k in kinds):
                worst = "ssd" if "ssd" in kinds else "unknown"
                log(reset.secure_erase_advice(worst))
                for rep_path in ssd_reps.values():
                    cmd = reset.windows_freespace_wipe_command(rep_path)
                    if cmd:
                        log("SSD on Windows — wiping free space: " + " ".join(cmd))
                        try:
                            code = reset.run_command(cmd)
                            log(f"free-space wipe exited with code {code}")
                        except Exception as e:  # noqa: BLE001
                            log(f"free-space wipe failed: {e}")
            else:  # all HDD
                log(reset.secure_erase_advice("hdd"))

    return report
