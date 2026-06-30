# SPDX-License-Identifier: GPL-3.0-or-later
"""Command-line interface for backuptool (Linux/macOS)."""
from __future__ import annotations

import argparse
import sys

from . import __version__, core
from . import crypto


def _ask_passphrase(confirm: bool) -> str:
    """Read the password from BACKUPTOOL_PASSWORD (automation) or prompt."""
    import os
    import getpass
    env = os.environ.get("BACKUPTOOL_PASSWORD")
    if env:
        return env
    try:
        p = getpass.getpass("Password: ")
        if confirm:
            if p != getpass.getpass("Repeat password: "):
                print("Passwords do not match.")
                sys.exit(1)
    except EOFError:
        print("No password provided (set BACKUPTOOL_PASSWORD or run interactively).")
        sys.exit(1)
    return p


def _cipher_and_pw(cipher_arg: str, confirm: bool):
    """Normalize the cipher, verify the encryption libraries are present (clean
    message if not), then obtain the passphrase. Returns (cipher, passphrase)."""
    cipher = crypto.normalize_cipher(cipher_arg)
    if cipher == crypto.CIPHER_NONE:
        return cipher, None
    try:
        crypto.ensure_available()
    except crypto.CryptoUnavailable as e:
        print(f"Error: {e}")
        sys.exit(1)
    return cipher, _ask_passphrase(confirm)


def _add_common(p):
    p.add_argument("-j", "--workers", type=int, default=core.default_workers(),
                   help="number of parallel workers (default: CPU cores)")
    p.add_argument("-n", "--dry-run", action="store_true", help="dry run")


def _vss_setup(sources):
    """Snapshot each source volume; return (scan_sources, vss_map, shadow_ids)."""
    from . import vss
    scan_sources, vmap, ids = vss.prepare(sources)
    for dev, vol in vmap:
        print(f"VSS snapshot of {vol}: {dev.rstrip(chr(92))}")
    return scan_sources, vmap, ids


def cmd_backup(a):
    import os
    if a.system is None:  # auto: include system dirs when running as root
        include_system = hasattr(os, "geteuid") and os.geteuid() == 0
    else:
        include_system = a.system
    sources = list(a.sources)
    if getattr(a, "auto", False):
        sources = core.auto_sources()
        if not sources:
            print("--auto found no user-data folders; pass sources explicitly.")
            sys.exit(1)
        print("Auto sources: " + ", ".join(sources))
    if not sources:
        print("Provide at least one source (or use --auto).")
        sys.exit(1)

    # Warn about / close apps that hold files open.
    from . import procs
    blockers = procs.running_blockers()
    if blockers:
        print("Apps that may lock files are running: " + ", ".join(blockers))
        if getattr(a, "close_apps", False):
            import time
            print("Asked to close: " + ", ".join(procs.close_apps()))
            time.sleep(2)
            still = procs.running_blockers()
            if still:
                print("Still running: " + ", ".join(still) + " — files they hold may still fail.")
        else:
            print("Close them, or use --close-apps / --vss, to avoid 'in use' errors.")

    cipher, passphrase = _cipher_and_pw(a.cipher, confirm=True)
    dest = a.dest or core.suggested_dest()
    if not a.dest:
        print(f"Using suggested destination: {dest}")

    scan_sources, vmap, shadow_ids = sources, None, []
    if getattr(a, "vss", False) and not a.dry_run:
        try:
            scan_sources, vmap, shadow_ids = _vss_setup(sources)
        except Exception as e:  # noqa: BLE001
            print(f"VSS unavailable ({e}); continuing without snapshot.")
            scan_sources, vmap, shadow_ids = sources, None, []
    elif getattr(a, "vss", False):
        print("--vss ignored in dry-run.")

    try:
        res = core.backup(
            sources=scan_sources, dest=dest, setname=a.set, workers=a.workers,
            use_checksum=a.checksum, extra_excludes=a.exclude, prune=a.delete,
            dry_run=a.dry_run, include_system=include_system, cipher=cipher,
            passphrase=passphrase, vss_map=vmap, verify=not getattr(a, "no_verify", False),
            log=print, progress=_progress if a.progress else None,
        )
    finally:
        if shadow_ids:
            from . import vss as _vss
            for sid in shadow_ids:
                _vss.remove(sid)
    return res


def cmd_restore(a):
    import os
    # Detect encryption from the manifest first -> ask for the password if needed.
    set_root = (os.path.join(a.source, a.set) if a.set else a.source)
    man = core.load_manifest(os.path.join(set_root, core.MANIFEST_NAME))
    needs_pw = bool(man) and crypto.normalize_cipher(man.get("cipher", "none")) != crypto.CIPHER_NONE
    if needs_pw:
        try:
            crypto.ensure_available()
        except crypto.CryptoUnavailable as e:
            print(f"Error: {e}")
            sys.exit(1)
    passphrase = _ask_passphrase(False) if needs_pw else None
    return core.restore(
        backup_dir=a.source, setname=a.set, target=a.target, workers=a.workers,
        reapply_meta=not a.no_meta, dry_run=a.dry_run, passphrase=passphrase,
        log=print, progress=_progress if a.progress else None,
    )


def cmd_list(a):
    sets = core.list_sets(a.dest)
    if not sets:
        print("No backup sets found in:", a.dest)
        return
    print(f"{'SET':24} {'HOST':16} {'CREATED':20} FILES")
    for s in sets:
        print(f"{s['set']:24} {s['host']:16} {s['created']:20} {s['files']}")


def cmd_decommission(a):
    import os
    from . import reset
    resolved = core.scope_sources(a.scope, a.sources)
    cipher, passphrase = _cipher_and_pw(a.cipher, confirm=True)
    dest = a.dest or core.suggested_dest()
    if not a.dest:
        print(f"Using suggested destination: {dest}")

    # Phase 2 reset actions both require clearing the source first.
    delete_source = a.delete_source or a.secure_wipe or bool(a.restore_defaults)

    if a.restore_defaults:
        action = f"wipe user data, then restore factory defaults from '{a.restore_defaults}'"
    elif a.secure_wipe:
        action = f"securely overwrite ({max(1, a.wipe_passes)} pass(es)) and delete the source files"
    elif delete_source:
        action = "delete the source files"
    else:
        action = "keep the source files (copy only)"

    # Warning + typed confirmation before any source deletion.
    if delete_source and not a.dry_run and not a.force:
        print(f"\n⚠️  WARNING — after a VERIFIED copy to '{dest}', this will:")
        print(f"        {action}")
        print("    in the ORIGINAL location(s):")
        for s in resolved:
            print(f"        {s}")
        print("    This is PERMANENT. (Nothing happens unless every file is copied AND SHA-256-verified.)")
        try:
            answer = input("\n  Type 'ERASE' to confirm, anything else to abort: ")
        except EOFError:
            answer = ""
        if answer.strip() != "ERASE":
            print("Aborted — confirmation 'ERASE' not given.")
            sys.exit(1)

    try:
        report = core.evacuate(
            sources=resolved, dest=dest, setname=a.set, workers=a.workers,
            extra_excludes=a.exclude, scope=a.scope, delete_source=delete_source,
            allow_same_device=a.allow_same_device, cipher=cipher,
            passphrase=passphrase, secure_wipe=a.secure_wipe,
            wipe_passes=a.wipe_passes, dry_run=a.dry_run, log=print,
            progress=_progress if a.progress else None,
        )
    except (RuntimeError, OSError, ValueError) as e:
        print(f"Error: {e}")
        sys.exit(1)
    print(f"Evacuation: {report['scanned']} scanned, {report['copied']} copied, "
          f"{report['verified']} verified, {report['deleted']} deleted "
          f"({report['wiped']} securely wiped, {report['delete_errors']} delete errors).")

    # Phase 2 action (b): restore a factory-default configuration after wiping.
    defaults_errors = 0
    if a.restore_defaults:
        if a.dry_run:
            print(f"[dry-run] would restore defaults from '{a.restore_defaults}' "
                  f"-> '{a.defaults_target}'.")
        elif report["delete_errors"] == 0:
            print(f"Restoring factory-default config from '{a.restore_defaults}' ...")
            set_root = (os.path.join(a.restore_defaults, a.defaults_set)
                        if a.defaults_set else a.restore_defaults)
            dman = core.load_manifest(os.path.join(set_root, core.MANIFEST_NAME))
            needs_pw = bool(dman) and crypto.normalize_cipher(
                dman.get("cipher", "none")) != crypto.CIPHER_NONE
            if needs_pw:
                try:
                    crypto.ensure_available()
                except crypto.CryptoUnavailable as e:
                    print(f"Error: {e}")
                    sys.exit(1)
            def_pw = _ask_passphrase(False) if needs_pw else None
            try:
                dstats = core.restore(
                    backup_dir=a.restore_defaults, setname=a.defaults_set,
                    target=a.defaults_target, workers=a.workers, reapply_meta=True,
                    dry_run=False, passphrase=def_pw, log=print,
                    progress=_progress if a.progress else None)
            except (RuntimeError, OSError, ValueError) as e:
                print(f"Error: {e}")
                sys.exit(1)
            defaults_errors = dstats.get("errors", 0)
            print(f"Defaults restored: {dstats.get('restored', 0)} files, "
                  f"{defaults_errors} errors.")
        else:
            print("Skipping defaults restore: source deletion had errors.")
            defaults_errors = 1

    # Phase 3 action (c): run the platform factory reset / a custom command.
    print("\n--- Factory reset ---")
    if a.reset_command:
        reset_argv = ["/bin/sh", "-c", a.reset_command]
    elif a.reset_os:
        reset_argv = reset.factory_reset_command()
    else:
        reset_argv = None

    if reset_argv:
        if a.dry_run:
            print(f"[dry-run] would run reset command: {' '.join(reset_argv)}")
        else:
            print(f"Running reset command: {' '.join(reset_argv)}")
            code = reset.run_command(reset_argv)
            print(f"Reset command exited with code {code}.")
    elif a.reset_os or a.reset_command:
        print("No automatic reset on this platform — do it manually:")
        print(reset.instructions())
    else:
        print(reset.instructions())

    if (report["delete_errors"] or report["verify_errors"]
            or report["backup_errors"] or defaults_errors):
        sys.exit(1)
    return report


def cmd_clone(a):
    from . import reset
    if a.list_tools:
        print("Clone tools on this system:")
        for t in reset.detect_clone_tools():
            mark = "✓" if t["available"] else "—"
            loc = t["path"] or "not installed"
            print(f"  [{mark}] {t['name']:<14} {t['description']}  ({loc})")
        return
    if not a.tool or not a.source or not a.target:
        print("Error: --tool, --source and --target are required (see --list-tools).")
        sys.exit(1)

    if reset.detect_clone_tools() and not _tool_available(reset, a.tool):
        print(f"Clone tool '{a.tool}' is not installed on this system.")
        print("Suggestion: install it, or pick an available one from --list-tools.")
        print("On platforms without a CLI cloner, use Clonezilla, Macrium Reflect or\n"
              "'Disk Utility' (macOS) / 'dd' from a live USB instead.")
        sys.exit(1)

    try:
        argv = reset.build_clone_command(a.tool, a.source, a.target)
    except ValueError as e:
        print(f"Error: {e}")
        sys.exit(1)
    if a.dry_run:
        print(f"[dry-run] would run: {' '.join(argv)}")
        return
    if not a.force:
        print(f"\n⚠️  WARNING — this will OVERWRITE the target '{a.target}' "
              f"with a clone of '{a.source}'.")
        try:
            answer = input("  Type 'CLONE' to confirm, anything else to abort: ")
        except EOFError:
            answer = ""
        if answer.strip() != "CLONE":
            print("Aborted — confirmation 'CLONE' not given.")
            sys.exit(1)
    print(f"Running: {' '.join(argv)}")
    code = reset.run_command(argv)
    print(f"Clone command exited with code {code}.")
    if code != 0:
        sys.exit(1)


def _tool_available(reset, name):
    import shutil
    return shutil.which(name) is not None


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
    b.add_argument("sources", nargs="*", help="source folders/files (or use --auto)")
    b.add_argument("-d", "--dest", default=None,
                   help="destination folder / backup drive "
                        "(default: suggested folder on the app's own drive)")
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
    b.add_argument("--cipher", default="none",
                   help="encryption: none | aes256gcm | chacha20poly1305")
    b.add_argument("--auto", action="store_true",
                   help="auto-pick this OS's user-data folders (Documents, Desktop, …)")
    b.add_argument("--vss", action="store_true",
                   help="Windows: snapshot the volume first (VSS) so locked/open files copy too")
    b.add_argument("--close-apps", action="store_true",
                   help="before backup, ask file-locking apps (browsers, Office, …) to close")
    b.add_argument("--no-verify", action="store_true",
                   help="skip reading the destination back to confirm the copy")
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

    d = sub.add_parser(
        "decommission",
        help="evacuate files to external storage (copy + verify, optional move), "
             "then print factory-reset guidance")
    d.add_argument("sources", nargs="*",
                   help="for scope=config (or extra mount points for scope=disk)")
    d.add_argument("--scope", choices=["home", "config", "disk"], default="home",
                   help="what to evacuate (default: home)")
    d.add_argument("-d", "--dest", default=None,
                   help="external destination (default: suggested folder on the app's "
                        "own drive) — must be on a DIFFERENT device than the source")
    d.add_argument("-s", "--set", default=None,
                   help="backup set name (default: <hostname>-decommission)")
    d.add_argument("-e", "--exclude", action="append", default=[],
                   help="additional exclude pattern (repeatable)")
    d.add_argument("--delete-source", action="store_true",
                   help="after a verified copy, DELETE the sources (turns copy into move)")
    d.add_argument("--secure-wipe", action="store_true",
                   help="securely overwrite source files before deleting "
                        "(implies --delete-source)")
    d.add_argument("--wipe-passes", type=int, default=1,
                   help="overwrite passes for --secure-wipe (default: 1)")
    d.add_argument("--restore-defaults", default=None, metavar="BACKUP_DIR",
                   help="after deletion, restore this backup folder as factory-default "
                        "config (implies --delete-source)")
    d.add_argument("--defaults-set", default=None,
                   help="set name inside --restore-defaults")
    d.add_argument("--defaults-target", default="/",
                   help="where to restore the defaults (default: /)")
    d.add_argument("--force", action="store_true",
                   help="skip the interactive 'ERASE' confirmation (for automation)")
    d.add_argument("--allow-same-device", action="store_true",
                   help="skip the different-device safety check (discouraged)")
    d.add_argument("--cipher", default="none",
                   help="encryption for the evacuated copy: none | aes256gcm | chacha20poly1305")
    d.add_argument("--reset-os", action="store_true",
                   help="after evacuation, run the platform factory reset (Windows) "
                        "or print manual instructions (macOS/Linux)")
    d.add_argument("--reset-command", default=None, metavar="CMD",
                   help="custom reset command to run after evacuation (appliances)")
    d.add_argument("--progress", action="store_true", help="show progress")
    _add_common(d)
    d.set_defaults(func=cmd_decommission)

    c = sub.add_parser("clone",
                       help="clone a disk/partition to an image or device using a detected tool")
    c.add_argument("--list-tools", action="store_true",
                   help="list detected clone tools on this system and exit")
    c.add_argument("--tool", default=None, help="clone tool to use (e.g. dd, ddrescue)")
    c.add_argument("--source", default=None, help="source device or image file")
    c.add_argument("--target", default=None,
                   help="target device or image file (WILL BE OVERWRITTEN)")
    c.add_argument("--force", action="store_true",
                   help="skip the typed 'CLONE' confirmation (for automation)")
    c.add_argument("-n", "--dry-run", action="store_true",
                   help="print the command instead of running it")
    c.set_defaults(func=cmd_clone)

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
