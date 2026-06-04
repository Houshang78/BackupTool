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
import shutil
import hashlib
import fnmatch
import socket
from concurrent.futures import ThreadPoolExecutor, ProcessPoolExecutor, as_completed

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
]


# ----------------------------------------------------------------------------
# Helpers
# ----------------------------------------------------------------------------
def default_workers() -> int:
    return max(1, (os.cpu_count() or 2))


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
        "mtime": st.st_mtime,
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
    rel = full.lstrip("/")
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
def copy_one(meta: dict, set_root: str, rel: str):
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
    shutil.copy2(meta["path"], dst, follow_symlinks=False)
    return meta["size"]


# ----------------------------------------------------------------------------
# Backup
# ----------------------------------------------------------------------------
def backup(sources, dest, setname=None, workers=None, use_checksum=False,
           extra_excludes=None, prune=False, dry_run=False,
           log=print, progress=None):
    workers = workers or default_workers()
    excludes = list(extra_excludes or []) + DEFAULT_EXCLUDES
    setname = setname or socket.gethostname()
    set_root = os.path.join(os.path.abspath(dest), setname)
    manifest_path = os.path.join(set_root, MANIFEST_NAME)

    prev = load_manifest(manifest_path)
    prev_files = prev.get("files", {}) if prev else {}

    log("Scanning sources ...")
    entries = scan(sources, excludes)
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
    total = len(todo)
    with ThreadPoolExecutor(max_workers=workers) as ex:
        futs = {ex.submit(copy_one, entries[r], set_root, r): r for r in todo}
        for fut in as_completed(futs):
            r = futs[fut]
            done += 1
            try:
                nbytes = fut.result()
                copied += 1
                copied_bytes += nbytes
            except Exception as e:
                errors += 1
                log(f"  ERROR {r}: {e}")
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
        "files": {r: {k: v for k, v in m.items() if k != "path"}
                  for r, m in entries.items()},
    }
    save_manifest(manifest_path, man, set_root)
    log(f"Done: {copied} copied ({human(copied_bytes)}), {unchanged} skipped, "
        f"{deleted} removed, {errors} errors.")
    return {"copied": copied, "skipped": unchanged, "bytes": copied_bytes,
            "deleted": deleted, "errors": errors}


# ----------------------------------------------------------------------------
# Restore
# ----------------------------------------------------------------------------
def _am_root() -> bool:
    return hasattr(os, "geteuid") and os.geteuid() == 0


def restore(backup_dir, setname=None, target="/", workers=None,
            reapply_meta=True, dry_run=False, log=print, progress=None):
    if setname:
        set_root = os.path.join(os.path.abspath(backup_dir), setname)
    else:
        set_root = os.path.abspath(backup_dir)
    manifest_path = os.path.join(set_root, MANIFEST_NAME)
    man = load_manifest(manifest_path)
    if not man:
        raise FileNotFoundError(f"No manifest at {manifest_path}")

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
        shutil.copy2(src, dst, follow_symlinks=False)
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
