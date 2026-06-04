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

# File kinds. Kept as the JSON strings "file"/"symlink"/"dir" so the manifest
# stays compatible with the Rust implementation.
KIND_FILE = "file"
KIND_SYMLINK = "symlink"
KIND_DIR = "dir"

# Default excludes (cache / trash / temporary)
DEFAULT_EXCLUDES = [
    "*/.cache/*", "*/.cache",
    "*/.local/share/Trash/*", "*/.local/share/Trash",
    "*/.thumbnails/*", "*/.thumbnails",
    "*/Cache/*", "*/Cache",
    "*/cache2/*",
    "*/.gvfs", "*.sock", "*/lost+found", "lost+found",
    "*/.Spotlight-V100/*", "*/.Trashes/*", "*/.fseventsd/*",
]


# ----------------------------------------------------------------------------
# Helpers
# ----------------------------------------------------------------------------
def default_workers() -> int:
    return max(1, (os.cpu_count() or 2))


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
    elif stat.S_ISDIR(mode):
        meta["type"] = KIND_DIR  # recorded so empty dirs and dir metadata survive
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
            _add_entry(entries, root, excludes)  # the directory itself (empty dirs too)
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
        dest = os.path.join(hist, f"manifest-{stamp}.json")
        n = 1
        while os.path.exists(dest):  # two runs in the same second must not overwrite
            dest = os.path.join(hist, f"manifest-{stamp}-{n}.json")
            n += 1
        shutil.copy2(manifest_path, dest)
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
                    "files": sum(1 for v in man.get("files", {}).values()
                                 if v.get("type") != KIND_DIR),
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
    if meta["type"] == KIND_DIR:
        os.makedirs(dst, exist_ok=True)
        return 0
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
           include_system=False, log=print, progress=None):
    workers = workers or default_workers()
    excludes = list(extra_excludes or []) + DEFAULT_EXCLUDES
    setname = setname or socket.gethostname()
    set_root = os.path.join(os.path.abspath(dest), setname)
    manifest_path = os.path.join(set_root, MANIFEST_NAME)

    sources = list(sources)
    if include_system:
        sysdirs = system_dirs()
        log(f"Including system directories: {', '.join(sysdirs) or '(none)'}")
        sources += sysdirs

    prev = load_manifest(manifest_path)
    prev_files = prev.get("files", {}) if prev else {}

    log("Scanning sources ...")
    entries = scan(sources, excludes)
    dir_rels = [r for r, m in entries.items() if m["type"] == KIND_DIR]
    file_entries = {r: m for r, m in entries.items() if m["type"] != KIND_DIR}
    log(f"{len(file_entries)} files, {len(dir_rels)} dirs found.")

    if use_checksum and file_entries:
        log(f"Computing SHA-256 with {workers} processes (multicore) ...")
        rels = list(file_entries.keys())
        paths = [entries[r]["path"] for r in rels]
        try:
            with ProcessPoolExecutor(max_workers=workers) as ex:
                for rel, digest in zip(rels, ex.map(sha256_file, paths, chunksize=8)):
                    entries[rel]["sha256"] = digest
        except Exception as e:  # fall back to single core
            log(f"  ProcessPool unavailable ({e}) – computing serially.")
            for rel in rels:
                entries[rel]["sha256"] = sha256_file(entries[rel]["path"])

    # Directories are tracked separately: they are always (re)created so empty
    # dirs survive, but never counted as changed/unchanged/copied bytes.
    todo = [r for r, m in file_entries.items()
            if needs_copy(m, prev_files.get(r), use_checksum)]
    unchanged = len(file_entries) - len(todo)
    total_bytes = sum(file_entries[r]["size"] for r in todo if file_entries[r]["type"] == KIND_FILE)
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
    # Recreate every source directory (shallow first) so empty dirs are preserved.
    for r in sorted(dir_rels):
        try:
            os.makedirs(os.path.join(set_root, r), exist_ok=True)
        except OSError:
            pass

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
        # Remove vanished files first, then now-empty dirs (deepest first).
        del_dirs = [r for r in deletions if prev_files.get(r, {}).get("type") == KIND_DIR]
        del_files = [r for r in deletions if prev_files.get(r, {}).get("type") != KIND_DIR]
        for r in del_files:
            tgt = os.path.join(set_root, r)
            try:
                if os.path.lexists(tgt):
                    os.remove(tgt)
                    deleted += 1
            except OSError:
                pass
        for r in sorted(del_dirs, reverse=True):
            tgt = os.path.join(set_root, r)
            try:
                if os.path.isdir(tgt):
                    os.rmdir(tgt)
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

    log(f"Done: {copied} copied ({human(copied_bytes)}), {unchanged} skipped, "
        f"{deleted} removed, {errors} errors.")
    return {"copied": copied, "skipped": unchanged, "bytes": copied_bytes,
            "deleted": deleted, "errors": errors, "log": logpath}


# ----------------------------------------------------------------------------
# Restore
# ----------------------------------------------------------------------------
def _am_root() -> bool:
    return hasattr(os, "geteuid") and os.geteuid() == 0


def _is_safe_rel(rel: str) -> bool:
    """Reject manifest paths that would escape the restore target (absolute
    paths or any '..' component). Manifests written by scan() are always safe;
    this guards against a tampered or hand-edited manifest."""
    if not rel or os.path.isabs(rel):
        return False
    return ".." not in rel.replace("\\", "/").split("/")


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
    for r in files:
        if not _is_safe_rel(r):
            log(f"  SKIP unsafe path in manifest: {r}")
    dir_items = [(r, m) for r, m in files.items()
                 if m.get("type") == KIND_DIR and _is_safe_rel(r)]
    file_items = [(r, m) for r, m in files.items()
                  if m.get("type") != KIND_DIR and _is_safe_rel(r)]
    total = len(file_items)
    am_root = _am_root()
    log(f"Restoring {len(file_items)} files, {len(dir_items)} dirs -> {target}")

    if dry_run:
        for rel, _ in (dir_items + file_items)[:1000]:
            log(f"  [would restore] {os.path.join(target, rel)}")
        return {"restored": 0, "dryrun": True}

    def _apply_meta(dst, meta):
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

    # Phase 1: create directories shallow first (metadata reapplied last).
    for rel, _ in sorted(dir_items):
        try:
            os.makedirs(os.path.join(target, rel), exist_ok=True)
        except OSError as e:
            log(f"  ERROR {rel}: {e}")

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
            _apply_meta(dst, meta)
        return meta.get("size", 0)

    # Phase 2: restore files and symlinks in parallel.
    done = restored = errors = 0
    with ThreadPoolExecutor(max_workers=workers) as ex:
        futs = {ex.submit(_restore_one, rel, meta): rel for rel, meta in file_items}
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

    # Phase 3: reapply directory metadata deepest first, so child writes from
    # phase 2 cannot clobber a directory's mtime afterwards.
    if reapply_meta:
        for rel, meta in sorted(dir_items, reverse=True):
            _apply_meta(os.path.join(target, rel), meta)

    log(f"Restore done: {restored} restored, {errors} errors.")
    if reapply_meta and not am_root:
        log("Note: without root, owner (uid/gid) could not be set – "
            "re-run with sudo if needed.")
    return {"restored": restored, "errors": errors}
