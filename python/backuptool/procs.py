# SPDX-License-Identifier: GPL-3.0-or-later
"""Detect (and optionally close) running applications that commonly hold user
files open, so a file-level backup doesn't fail with "in use" errors.
Cross-platform: tasklist on Windows, ps elsewhere.
"""
from __future__ import annotations

import subprocess
import sys

# (process token, friendly name)
BLOCKERS = [
    ("firefox", "Firefox"), ("chrome", "Google Chrome"), ("msedge", "Microsoft Edge"),
    ("chromium", "Chromium"), ("brave", "Brave"), ("opera", "Opera"),
    ("vivaldi", "Vivaldi"), ("safari", "Safari"), ("thunderbird", "Thunderbird"),
    ("photoshop", "Photoshop"), ("illustrator", "Illustrator"), ("indesign", "InDesign"),
    ("lightroom", "Lightroom"), ("figma", "Figma"), ("acrobat", "Acrobat"),
    ("acrord32", "Acrobat Reader"), ("winword", "Word"), ("excel", "Excel"),
    ("powerpnt", "PowerPoint"), ("outlook", "Outlook"), ("onenote", "OneNote"),
    ("onedrive", "OneDrive"), ("dropbox", "Dropbox"), ("spotify", "Spotify"),
]


def _process_list_lower() -> str:
    try:
        if sys.platform.startswith("win"):
            out = subprocess.run(["tasklist"], capture_output=True, text=True, timeout=30)
        else:
            out = subprocess.run(["ps", "-A", "-o", "comm="], capture_output=True, text=True, timeout=30)
        return out.stdout.lower()
    except OSError:
        return ""


def _token_running(listing: str, token: str) -> bool:
    if sys.platform.startswith("win"):
        return (token + ".exe") in listing
    return token in listing


def running_blockers():
    """Friendly names of running apps likely to lock files (deduplicated)."""
    listing = _process_list_lower()
    found = []
    for tok, name in BLOCKERS:
        if _token_running(listing, tok) and name not in found:
            found.append(name)
    return found


def close_apps():
    """Ask each running blocker to close (graceful). Returns the names signalled."""
    listing = _process_list_lower()
    closed = []
    for tok, name in BLOCKERS:
        if not _token_running(listing, tok):
            continue
        try:
            if sys.platform.startswith("win"):
                subprocess.run(["taskkill", "/IM", tok + ".exe"], capture_output=True, text=True)
            else:
                subprocess.run(["pkill", "-i", tok], capture_output=True, text=True)
        except OSError:
            pass
        if name not in closed:
            closed.append(name)
    return closed
