# SPDX-License-Identifier: GPL-3.0-or-later
"""Command-line interface for backuptool (Linux/macOS)."""
from __future__ import annotations

import argparse
import sys

from . import __version__, core


def _add_common(p):
    p.add_argument("-j", "--workers", type=int, default=core.default_workers(),
                   help="number of parallel workers (default: CPU cores)")
    p.add_argument("-n", "--dry-run", action="store_true", help="dry run")


def cmd_backup(a):
    import os
    if a.system is None:  # auto: include system dirs when running as root
        include_system = hasattr(os, "geteuid") and os.geteuid() == 0
    else:
        include_system = a.system
    return core.backup(
        sources=a.sources, dest=a.dest, setname=a.set, workers=a.workers,
        use_checksum=a.checksum, extra_excludes=a.exclude, prune=a.delete,
        dry_run=a.dry_run, include_system=include_system, log=print,
        progress=_progress if a.progress else None,
    )


def cmd_restore(a):
    return core.restore(
        backup_dir=a.source, setname=a.set, target=a.target, workers=a.workers,
        reapply_meta=not a.no_meta, dry_run=a.dry_run, log=print,
        progress=_progress if a.progress else None,
    )


def cmd_list(a):
    sets = core.list_sets(a.dest)
    if not sets:
        print("No backup sets found in:", a.dest)
        return
    print(f"{'SET':24} {'HOST':16} {'CREATED':20} FILES")
    for s in sets:
        print(f"{s['set']:24} {s['host']:16} {s['created']:20} {s['files']}")


def cmd_discover(a):
    from . import discover as dsc
    print("Sources (auto-detected):")
    for c in dsc.user_data_sources():
        print(f"  {c['kind']:8} {c['path']}")
    print("Destinations (removable / network):")
    dests = dsc.detect_destinations()
    if not dests:
        print("  (none mounted)")
    for c in dests:
        print(f"  {c['kind']:8} {c['path']}")
    print("Suggested destination:", dsc.default_destination() or "(none)")


def _progress(done, total, path):
    pct = (done / total * 100) if total else 100
    sys.stdout.write(f"\r  {done}/{total} ({pct:5.1f}%) {path[:60]:60}")
    sys.stdout.flush()
    if done == total:
        sys.stdout.write("\n")


def build_parser():
    p = argparse.ArgumentParser(
        prog="backuptool",
        description="Cross-platform, parallel, incremental backup tool.")
    p.add_argument("--version", action="version", version=f"backuptool {__version__}")
    sub = p.add_subparsers(dest="cmd", required=True)

    b = sub.add_parser("backup", help="back up")
    b.add_argument("sources", nargs="+", help="source folders/files")
    b.add_argument("-d", "--dest", required=True, help="destination folder / backup drive")
    b.add_argument("-s", "--set", default=None,
                   help="backup set name (default: hostname) – for multiple systems")
    b.add_argument("-c", "--checksum", action="store_true",
                   help="compare by SHA-256 instead of mtime/size (multicore)")
    b.add_argument("-e", "--exclude", action="append", default=[],
                   help="additional exclude pattern (repeatable)")
    b.add_argument("--delete", action="store_true",
                   help="delete in destination what was removed from source (mirror)")
    b.add_argument("--system", dest="system", action="store_true", default=None,
                   help="also back up system dirs (/etc, ...); automatic as root")
    b.add_argument("--no-system", dest="system", action="store_false",
                   help="do not include system dirs even when running as root")
    b.add_argument("--progress", action="store_true", help="show progress")
    _add_common(b)
    b.set_defaults(func=cmd_backup)

    r = sub.add_parser("restore", help="restore")
    r.add_argument("-S", "--source", required=True, help="backup folder")
    r.add_argument("-s", "--set", default=None, help="backup set (hostname)")
    r.add_argument("-t", "--target", default="/", help="target root (default: /)")
    r.add_argument("--no-meta", action="store_true",
                   help="do NOT reapply permissions/owner")
    r.add_argument("--progress", action="store_true", help="show progress")
    _add_common(r)
    r.set_defaults(func=cmd_restore)

    li = sub.add_parser("list", help="list backup sets")
    li.add_argument("-d", "--dest", required=True, help="destination folder")
    li.set_defaults(func=cmd_list)

    sub.add_parser("discover", help="list auto-detected sources and destinations") \
       .set_defaults(func=cmd_discover)

    g = sub.add_parser("gui", help="launch the graphical interface")
    g.set_defaults(func=lambda a: _launch_gui())
    return p


def _launch_gui():
    from . import gui
    gui.main()


def main(argv=None):
    args = build_parser().parse_args(argv)
    res = args.func(args)
    if isinstance(res, dict) and res.get("errors"):
        sys.exit(1)


if __name__ == "__main__":
    main()
