# SPDX-License-Identifier: GPL-3.0-or-later
"""Discovery helpers: find user/service data to back up and good destinations.

Platform-specific, standard library only. Each candidate is a dict:
    {"label": str, "path": str, "kind": str}

Source kinds:  user | service | data | config
Dest kinds:    usb | external | network
"""
from __future__ import annotations

import os
import sys

# Login shells that mark an account as "no real data" on Linux.
_NOLOGIN = {"/usr/sbin/nologin", "/sbin/nologin", "/bin/false", "/usr/bin/false", ""}
_TRIVIAL_HOMES = {"/", "/nonexistent", "/dev/null", "/bin", "/sbin", "/usr/sbin", "/var/empty"}
_NET_FS = {"nfs", "nfs4", "cifs", "smbfs", "smb3", "fuse.sshfs", "ncpfs", "afs", "9p"}


def _dedup(cands: list) -> list:
    out, seen = [], set()
    for c in cands:
        p = os.path.realpath(c["path"])
        if p in seen or not os.path.isdir(p):
            continue
        seen.add(p)
        c["path"] = p
        out.append(c)
    return out


# ---------------------------------------------------------------- Linux
def _linux_sources() -> list:
    import pwd
    cands = []
    for e in pwd.getpwall():
        if 1000 <= e.pw_uid < 65534 and e.pw_dir.startswith("/home"):
            cands.append({"label": f"User: {e.pw_name}", "path": e.pw_dir, "kind": "user"})
    cands.append({"label": "root", "path": "/root", "kind": "user"})
    # service accounts with real data directories (postgres, www-data, ...)
    for e in pwd.getpwall():
        if (e.pw_uid < 1000 and e.pw_shell not in _NOLOGIN
                and e.pw_dir not in _TRIVIAL_HOMES
                and e.pw_dir.startswith(("/var", "/srv", "/opt"))):
            cands.append({"label": f"Service: {e.pw_name}", "path": e.pw_dir, "kind": "service"})
    for p in ("/srv", "/var/www", "/opt"):
        cands.append({"label": p, "path": p, "kind": "data"})
    cands.append({"label": "System config (/etc)", "path": "/etc", "kind": "config"})
    return _dedup(cands)


def _linux_destinations() -> list:
    cands = []
    try:
        with open("/proc/mounts", encoding="utf-8") as f:
            rows = [ln.split() for ln in f]
    except OSError:
        rows = []

    def removable(dev: str) -> bool:
        base = os.path.basename(dev).rstrip("0123456789")
        try:
            with open(f"/sys/block/{base}/removable", encoding="utf-8") as f:
                return f.read().strip() == "1"
        except OSError:
            return False

    for parts in rows:
        if len(parts) < 3:
            continue
        dev, mnt, fstype = parts[0], parts[1].replace("\\040", " "), parts[2]
        if fstype in _NET_FS:
            cands.append({"label": f"Network: {mnt}", "path": mnt, "kind": "network"})
        elif mnt.startswith(("/media/", "/run/media/", "/mnt/")) and dev.startswith("/dev/"):
            usb = removable(dev)
            cands.append({"label": f"{'USB' if usb else 'Disk'}: {mnt}",
                          "path": mnt, "kind": "usb" if usb else "external"})
    return _dedup(cands)


# ---------------------------------------------------------------- macOS
def _macos_sources() -> list:
    cands = []
    for name in sorted(os.listdir("/Users")) if os.path.isdir("/Users") else []:
        if name in ("Shared", "Guest", ".localized"):
            continue
        cands.append({"label": f"User: {name}", "path": f"/Users/{name}", "kind": "user"})
    for p in ("/usr/local/var", "/opt/homebrew/var", "/Library", "/srv"):
        cands.append({"label": p, "path": p, "kind": "data"})
    cands.append({"label": "System config (/etc)", "path": "/etc", "kind": "config"})
    return _dedup(cands)


def _macos_destinations() -> list:
    cands = []
    root_dev = os.stat("/").st_dev
    for name in sorted(os.listdir("/Volumes")) if os.path.isdir("/Volumes") else []:
        path = f"/Volumes/{name}"
        try:
            if os.stat(path).st_dev == root_dev:
                continue  # the boot volume
        except OSError:
            continue
        cands.append({"label": f"Volume: {name}", "path": path, "kind": "external"})
    return _dedup(cands)


# ---------------------------------------------------------------- Windows
def _windows_sources() -> list:
    cands = []
    base = os.environ.get("SystemDrive", "C:") + "\\Users"
    for name in sorted(os.listdir(base)) if os.path.isdir(base) else []:
        if name.lower() in ("public", "default", "default user", "all users"):
            continue
        cands.append({"label": f"User: {name}", "path": os.path.join(base, name), "kind": "user"})
    pd = os.environ.get("ProgramData")
    if pd:
        cands.append({"label": "ProgramData", "path": pd, "kind": "data"})
    return _dedup(cands)


def _windows_destinations() -> list:
    import ctypes
    cands = []
    DRIVE_REMOVABLE, DRIVE_REMOTE = 2, 4
    k32 = ctypes.windll.kernel32
    mask = k32.GetLogicalDrives()
    for i in range(26):
        if not (mask >> i) & 1:
            continue
        root = f"{chr(65 + i)}:\\"
        t = k32.GetDriveTypeW(ctypes.c_wchar_p(root))
        if t == DRIVE_REMOVABLE:
            cands.append({"label": f"USB: {root}", "path": root, "kind": "usb"})
        elif t == DRIVE_REMOTE:
            cands.append({"label": f"Network: {root}", "path": root, "kind": "network"})
    return cands


# ---------------------------------------------------------------- public API
def user_data_sources() -> list:
    """Candidate source directories worth backing up on this system."""
    if sys.platform.startswith("linux"):
        return _linux_sources()
    if sys.platform == "darwin":
        return _macos_sources()
    if os.name == "nt":
        return _windows_sources()
    return []


def detect_destinations() -> list:
    """Mounted USB/external disks and network shares that could hold the backup."""
    if sys.platform.startswith("linux"):
        return _linux_destinations()
    if sys.platform == "darwin":
        return _macos_destinations()
    if os.name == "nt":
        return _windows_destinations()
    return []


def resolve_uid(uid_or_name) -> str:
    """Resolve a UID or username to its home/data directory ('' if none).
    Used to also back up another user's (or a service account's) data."""
    if not sys.platform.startswith(("linux", "darwin")):
        return ""
    import pwd
    try:
        s = str(uid_or_name).strip()
        e = pwd.getpwuid(int(s)) if s.isdigit() else pwd.getpwnam(s)
    except (KeyError, ValueError):
        return ""
    return e.pw_dir if os.path.isdir(e.pw_dir) else ""


def default_destination() -> str:
    """Best default destination: USB first, then external disk, then network."""
    dests = detect_destinations()
    for kind in ("usb", "external", "network"):
        for c in dests:
            if c["kind"] == kind:
                return c["path"]
    return ""
