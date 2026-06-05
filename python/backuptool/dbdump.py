# SPDX-License-Identifier: GPL-3.0-or-later
"""Database dump adapters.

Detect locally installed databases and dump them into the backup set's
``.backuptool-db/`` folder. Native adapters (run as root via the local socket):
PostgreSQL, MySQL/MariaDB, Redis, MongoDB. Anything else (Oracle, MS SQL, ...)
is covered by a *generic* adapter: the user supplies a dump command; it runs
with ``BACKUPTOOL_DB_OUT`` pointing at the output folder.
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys

DB_DIR = ".backuptool-db"


def _has(binary: str) -> bool:
    return shutil.which(binary) is not None


def _am_root() -> bool:
    return hasattr(os, "geteuid") and os.geteuid() == 0


def detect_databases() -> list:
    """Installed databases we can dump. Each: {"name", "kind"}."""
    out = []
    if not sys.platform.startswith(("linux", "darwin")):
        return out
    if _has("pg_dumpall"):
        out.append({"name": "postgresql", "kind": "postgresql"})
    if _has("mysqldump") or _has("mariadb-dump"):
        out.append({"name": "mysql", "kind": "mysql"})
    if _has("redis-cli"):
        out.append({"name": "redis", "kind": "redis"})
    if _has("mongodump"):
        out.append({"name": "mongodb", "kind": "mongodb"})
    return out


def _native_command(kind: str, out_dir: str):
    """Return (argv, stdout_path_or_None) for a built-in adapter."""
    if kind == "postgresql":
        if _am_root():
            runner = ["runuser", "-u", "postgres", "--"]
        elif _has("sudo"):
            runner = ["sudo", "-n", "-u", "postgres"]
        else:
            runner = []
        return runner + ["pg_dumpall"], os.path.join(out_dir, "postgresql-all.sql")
    if kind == "mysql":
        dump = "mariadb-dump" if _has("mariadb-dump") else "mysqldump"
        return ([dump, "--all-databases", "--single-transaction", "--routines", "--events"],
                os.path.join(out_dir, "mysql-all.sql"))
    if kind == "redis":
        return ["redis-cli", "--rdb", os.path.join(out_dir, "redis-dump.rdb")], None
    if kind == "mongodb":
        return ["mongodump", "--out", os.path.join(out_dir, "mongodb")], None
    return None, None


def dump_database(spec: dict, out_dir: str, log=print) -> bool:
    """Dump one database. ``spec`` is either {"kind": <builtin>} or
    {"name", "shell": <command>} for a generic/custom dump (e.g. Oracle)."""
    os.makedirs(out_dir, exist_ok=True)
    name = spec.get("name", spec.get("kind", "db"))

    if spec.get("shell"):
        outfile = os.path.join(out_dir, f"{name}.log")
        env = dict(os.environ, BACKUPTOOL_DB_OUT=out_dir)
        try:
            with open(outfile, "w", encoding="utf-8") as f:
                rc = subprocess.run(spec["shell"], shell=True, env=env,
                                    stdout=f, stderr=subprocess.STDOUT).returncode
        except OSError as e:
            log(f"  DB {name}: error {e}")
            return False
        log(f"  DB {name}: {'ok' if rc == 0 else f'FAILED (exit {rc})'}")
        return rc == 0

    argv, stdout_path = _native_command(spec["kind"], out_dir)
    if not argv:
        log(f"  DB {name}: no adapter")
        return False
    try:
        if stdout_path:
            with open(stdout_path, "w", encoding="utf-8") as f:
                p = subprocess.run(argv, stdout=f, stderr=subprocess.PIPE)
        else:
            p = subprocess.run(argv, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    except OSError as e:
        log(f"  DB {name}: error {e}")
        return False
    if p.returncode == 0:
        log(f"  DB {name}: ok")
        return True
    err = p.stderr.decode("utf-8", "replace").strip().splitlines()
    log(f"  DB {name}: FAILED — {err[-1] if err else 'exit ' + str(p.returncode)}")
    return False


def dump_all(specs: list, out_dir: str, log=print) -> int:
    """Dump every spec into out_dir; return the number that succeeded."""
    ok = 0
    for spec in specs:
        if dump_database(spec, out_dir, log):
            ok += 1
    return ok
