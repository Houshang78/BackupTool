# SPDX-License-Identifier: GPL-3.0-or-later
"""Factory-reset guidance and Phase 3 actions.

- :func:`instructions`: platform-specific manual factory-reset steps.
- :func:`detect_clone_tools` / :func:`build_clone_command`: detect installed
  disk-clone utilities and build the command line for the chosen one.
- :func:`factory_reset_command`: the platform's automatable factory-reset
  command, when one exists (Windows); ``None`` means "do it manually".
- :func:`run_command`: spawn an external command (used for clone / reset).

Anything destructive is invoked by the CLI only behind a typed confirmation,
and can be previewed with ``--dry-run`` (which prints the exact command).
"""
from __future__ import annotations

import os
import sys
import shutil
import subprocess


def _clone_candidates():
    if sys.platform == "darwin":
        return [
            ("asr", "Apple Software Restore — block-copy/restore a volume"),
            ("dd", "raw byte copy of a device or file"),
            ("ddrescue", "GNU ddrescue — robust copy, retries bad sectors"),
        ]
    if sys.platform.startswith("linux"):
        return [
            ("dd", "raw byte copy of a device or file"),
            ("ddrescue", "GNU ddrescue — robust copy, retries bad sectors"),
            ("partclone.dd", "partclone — filesystem-aware partition clone"),
            ("ntfsclone", "clone NTFS partitions"),
            ("e2image", "clone ext2/3/4 partitions"),
        ]
    if sys.platform.startswith("win"):
        return [
            ("wbadmin", "Windows Server Backup — system image"),
            ("dd", "raw byte copy (if a dd port is installed)"),
        ]
    return []


def detect_clone_tools():
    """Return [{name, description, path, available}] for this platform."""
    out = []
    for name, desc in _clone_candidates():
        path = shutil.which(name)
        out.append({"name": name, "description": desc,
                    "path": path, "available": path is not None})
    return out


def build_clone_command(tool: str, source: str, target: str):
    """Build the argv to clone ``source`` to ``target`` with the named tool.
    ``source``/``target`` are devices or image files. Raises on unknown tool."""
    if tool == "dd":
        return ["dd", f"if={source}", f"of={target}", "bs=1048576", "conv=noerror,sync"]
    if tool == "ddrescue":
        return ["ddrescue", "-f", source, target, f"{target}.mapfile"]
    if tool == "asr":
        return ["asr", "restore", "--source", source, "--target", target,
                "--erase", "--noprompt"]
    if tool == "partclone.dd":
        return ["partclone.dd", "-s", source, "-o", target]
    if tool == "ntfsclone":
        return ["ntfsclone", "--save-image", "-o", target, source]
    if tool == "e2image":
        return ["e2image", "-r", source, target]
    if tool == "wbadmin":
        return ["wbadmin", "start", "backup", f"-backupTarget:{target}",
                f"-include:{source}", "-quiet"]
    raise ValueError(f"unknown clone tool '{tool}' (see --list-tools)")


def factory_reset_command():
    """The platform's automatable factory-reset command, or None when a reset
    can only be performed manually (see :func:`instructions`)."""
    if sys.platform.startswith("win"):
        return ["systemreset.exe", "--factoryreset"]
    return None


def run_command(argv) -> int:
    """Spawn an external command (inheriting stdio) and return its exit code."""
    if not argv:
        raise ValueError("empty command")
    try:
        return subprocess.call(argv)
    except OSError as e:
        raise RuntimeError(f"failed to run '{argv[0]}': {e}") from e


# ----------------------------------------------------------------------------
# Storage type detection (SSD vs HDD)
# ----------------------------------------------------------------------------
def storage_type(path: str) -> str:
    """Storage type backing ``path``: 'ssd' | 'hdd' | 'unknown'."""
    return storage_info(path)[1]


def storage_info(path: str):
    """Detect the partition/volume backing ``path`` and its storage type, as
    ``(device, kind)``. ``device`` names the partition (e.g. '/dev/disk2s4',
    '/dev/sda1', or a Windows drive) so callers can group files per partition on
    machines with several (possibly mixed SSD/HDD) disks. Matters for secure
    deletion: overwrite only reliably destroys data on HDDs."""
    try:
        if sys.platform.startswith("win"):
            return _storage_windows(path)
        if sys.platform.startswith("linux"):
            return _storage_linux(path)
        if sys.platform == "darwin":
            return _storage_macos(path)
    except Exception:
        return ("", "unknown")
    return ("", "unknown")


def _run_out(argv, timeout=30):
    return subprocess.run(argv, capture_output=True, text=True, timeout=timeout).stdout


def _storage_macos(path: str):
    df = _run_out(["df", os.path.abspath(path)]).splitlines()
    dev = df[1].split()[0] if len(df) >= 2 else ""
    info = _run_out(["diskutil", "info", dev or os.path.abspath(path)])
    for line in info.lower().splitlines():
        if "solid state" in line:
            return (dev, "ssd" if "yes" in line else "hdd")
    return (dev, "unknown")


def _storage_linux(path: str):
    src = _run_out(["df", "--output=source", os.path.abspath(path)]).splitlines()
    if len(src) < 2:
        return ("", "unknown")
    dev = src[1].strip()                          # the partition, e.g. /dev/sda1
    name = dev[len("/dev/"):] if dev.startswith("/dev/") else dev
    # strip partition suffix to the base block device
    if (name.startswith("nvme") or name.startswith("mmcblk")) and "p" in name:
        name = name[:name.rfind("p")]
    else:
        name = name.rstrip("0123456789")
    try:
        with open(f"/sys/block/{name}/queue/rotational") as f:
            return (dev, "hdd" if f.read().strip() == "1" else "ssd")
    except OSError:
        return (dev, "unknown")


def _storage_windows(path: str):
    drive = os.path.splitdrive(os.path.abspath(path))[0].rstrip(":")
    ps = ("$ErrorActionPreference='SilentlyContinue';"
          f"(Get-Partition -DriveLetter '{drive}' | Get-Disk | Get-PhysicalDisk).MediaType")
    s = _run_out(["powershell", "-NoProfile", "-Command", ps]).lower()
    kind = "ssd" if "ssd" in s else ("hdd" if "hdd" in s else "unknown")
    return (f"{drive}:", kind)


def secure_erase_advice(kind: str) -> str:
    """How to securely erase given the detected storage type."""
    head = {
        "hdd": "HDD detected — multi-pass overwrite reliably destroys the data.",
        "ssd": "SSD/flash detected — per-file overwrite is NOT reliable (wear-leveling keeps old copies).",
    }.get(kind, "Storage type unknown — treat overwrite as best-effort only.")
    if kind == "hdd":
        return head
    if sys.platform.startswith("win"):
        tool = ("Windows: run 'cipher /w:<volume>' to wipe free space, turn on BitLocker "
                "(crypto-erase),\n  or use the drive vendor's ATA Secure Erase utility.")
    elif sys.platform.startswith("linux"):
        tool = ("Linux: use 'blkdiscard' (TRIM), 'hdparm --security-erase', or 'nvme format -s2' "
                "on the whole device;\n  or full-disk encryption (LUKS) + key destruction.")
    elif sys.platform == "darwin":
        tool = ("macOS: use 'diskutil secureErase', or rely on FileVault crypto-erase "
                "('Erase All Content and Settings').")
    else:
        tool = "Use your platform's hardware secure-erase tool."
    return f"{head}\n  {tool}"


def windows_freespace_wipe_command(path: str):
    """On Windows, the built-in command to overwrite a volume's free space
    ('cipher /w'). None on other platforms (they have their own tools)."""
    if sys.platform.startswith("win"):
        return ["cipher", f"/w:{path}"]
    return None


def instructions() -> str:
    """Human-readable, platform-specific factory-reset instructions."""
    if sys.platform == "darwin":
        return (
            "macOS: a factory reset cannot be scripted by third-party tools.\n"
            "Do it manually:\n"
            "- Apple silicon / T2 Macs: System Settings > General > Transfer or Reset\n"
            "  > Erase All Content and Settings.\n"
            "- Older Macs: reboot into Recovery (hold Cmd-R), use Disk Utility to erase\n"
            "  the system volume, then reinstall macOS."
        )
    if sys.platform.startswith("linux"):
        return (
            "Linux: there is no universal factory reset.\n"
            "Typical options:\n"
            "- Appliance/immutable systems: trigger the vendor reset (e.g. reset an\n"
            "  OverlayFS upper layer, or re-flash the factory image).\n"
            "- Desktop installs: reinstall from your installation medium, or remove\n"
            "  user data (/home, /root) and reset configuration to defaults."
        )
    if sys.platform.startswith("win"):
        return (
            "Windows: use the built-in reset.\n"
            "- GUI: Settings > System > Recovery > Reset this PC > Remove everything.\n"
            "- CLI (elevated): systemreset.exe --factoryreset"
        )
    return "Factory reset is platform-specific; consult your device documentation."
