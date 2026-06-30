# SPDX-License-Identifier: GPL-3.0-or-later
"""Windows Volume Shadow Copy (VSS) — snapshot a volume so the file backup can
read files that are locked/open by other processes. No-op on other platforms.

Creating a shadow copy needs Administrator rights.
"""
from __future__ import annotations

import subprocess
import sys


def create(volume: str):
    """Create a shadow copy of `volume` (e.g. 'C:'). Returns {id, device, volume}.
    Raises RuntimeError on failure (no admin, not Windows, …)."""
    if not sys.platform.startswith("win"):
        raise RuntimeError("VSS snapshots are only available on Windows")
    drive = volume.rstrip("\\/")
    ps = ("$ErrorActionPreference='Stop';"
          f"$r=([wmiclass]'Win32_ShadowCopy').Create('{drive}\\','ClientAccessible');"
          "$s=Get-WmiObject Win32_ShadowCopy | Where-Object { $_.ID -eq $r.ShadowID };"
          "Write-Output ($s.ID + '|' + $s.DeviceObject)")
    try:
        out = subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", ps],
                             capture_output=True, text=True, timeout=120)
    except OSError as e:
        raise RuntimeError(f"could not run powershell: {e}") from e
    if out.returncode != 0:
        raise RuntimeError(f"VSS snapshot of {drive} failed (needs admin): {out.stderr.strip()}")
    line = next((l for l in out.stdout.splitlines() if "|" in l), "")
    sid, _, device = line.strip().partition("|")
    if not sid or not device:
        raise RuntimeError(f"VSS: could not parse snapshot output: {out.stdout.strip()}")
    return {"id": sid, "device": device, "volume": drive}


def volume_of(path: str):
    """Drive-letter volume of a path ('C:\\Users' -> 'C:'), or None."""
    if len(path) >= 2 and path[1] == ":" and path[0].isalpha():
        return path[0] + ":"
    return None


def prepare(sources):
    """Snapshot each distinct source volume and remap sources to read from the
    snapshot. Returns (scan_sources, vss_map, shadow_ids). Raises on failure."""
    shadows = {}
    created, new_sources, vmap = [], [], []
    for s in sources:
        vol = volume_of(s)
        if vol:
            sh = shadows.get(vol)
            if sh is None:
                sh = create(vol)
                shadows[vol] = sh
                created.append(sh["id"])
                vmap.append((sh["device"] + "\\", vol))
            suffix = s[len(vol):].lstrip("\\/")
            new_sources.append(sh["device"] + "\\" + suffix)
        else:
            new_sources.append(s)
    return new_sources, vmap, created


def remove(shadow_id: str) -> None:
    if not sys.platform.startswith("win"):
        return
    ps = f"(Get-WmiObject Win32_ShadowCopy | Where-Object {{ $_.ID -eq '{shadow_id}' }}).Delete()"
    try:
        subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", ps],
                       capture_output=True, text=True, timeout=60)
    except OSError:
        pass
