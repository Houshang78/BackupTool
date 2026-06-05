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
import socket as _socket
import subprocess
import sys

DB_DIR = ".backuptool-db"

_PORTS = {"postgresql": 5432, "mysql": 3306, "redis": 6379, "mongodb": 27017}
_SOCKETS = {
    "postgresql": ["/var/run/postgresql/.s.PGSQL.5432", "/tmp/.s.PGSQL.5432"],
    "mysql": ["/run/mysqld/mysqld.sock", "/var/run/mysqld/mysqld.sock", "/tmp/mysql.sock"],
    "redis": ["/var/run/redis/redis-server.sock", "/run/redis/redis-server.sock"],
    "mongodb": ["/tmp/mongodb-27017.sock"],
}


def _has(binary: str) -> bool:
    return shutil.which(binary) is not None


def _am_root() -> bool:
    return hasattr(os, "geteuid") and os.geteuid() == 0


def is_running(kind: str, conn: dict = None) -> bool:
    """Best-effort check that the database is actually accepting connections
    (TCP port probe, then default/explicit unix-socket file)."""
    conn = conn or {}
    host = conn.get("host") or "127.0.0.1"
    port = conn.get("port") or _PORTS.get(kind)
    if port:
        try:
            with _socket.create_connection((host, int(port)), timeout=0.4):
                return True
        except OSError:
            pass
    socks = list(_SOCKETS.get(kind, []))
    if conn.get("socket"):
        socks.insert(0, conn["socket"])
    return any(os.path.exists(s) for s in socks)


def detect_databases() -> list:
    """Installed databases we can dump. Each: {"name", "kind", "running"}."""
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
    for d in out:
        d["running"] = is_running(d["kind"])
    return out


def _native_command(kind: str, out_dir: str, conn: dict = None):
    """Return (argv, stdout_path_or_None) for a built-in adapter, applying the
    optional connection settings (host/port/socket/user)."""
    conn = conn or {}
    host, port = conn.get("host"), conn.get("port")
    user, sock, pw = conn.get("user"), conn.get("socket"), conn.get("password")
    if kind == "postgresql":
        runner = (["runuser", "-u", "postgres", "--"] if _am_root()
                  else ["sudo", "-n", "-u", "postgres"] if _has("sudo") else [])
        cmd = runner + ["pg_dumpall"]
        if host: cmd += ["-h", host]
        if port: cmd += ["-p", str(port)]
        if user: cmd += ["-U", user]
        return cmd, os.path.join(out_dir, "postgresql-all.sql")
    if kind == "mysql":
        dump = "mariadb-dump" if _has("mariadb-dump") else "mysqldump"
        cmd = [dump, "--all-databases", "--single-transaction", "--routines", "--events"]
        if host: cmd += ["-h", host]
        if port: cmd += ["-P", str(port)]
        if user: cmd += ["-u", user]
        if sock: cmd += ["--socket", sock]
        return cmd, os.path.join(out_dir, "mysql-all.sql")
    if kind == "redis":
        cmd = ["redis-cli"]
        if host: cmd += ["-h", host]
        if port: cmd += ["-p", str(port)]
        if sock: cmd += ["-s", sock]
        if pw: cmd += ["-a", pw]
        cmd += ["--rdb", os.path.join(out_dir, "redis-dump.rdb")]
        return cmd, None
    if kind == "mongodb":
        cmd = ["mongodump"]
        if host: cmd += ["--host", host]
        if port: cmd += ["--port", str(port)]
        if user: cmd += ["-u", user]
        if pw: cmd += ["-p", pw]
        cmd += ["--out", os.path.join(out_dir, "mongodb")]
        return cmd, None
    return None, None


def dump_database(spec: dict, out_dir: str, log=print, conn: dict = None) -> bool:
    """Dump one database. ``spec`` is either {"kind": <builtin>} or
    {"name", "shell": <command>} for a generic/custom dump (e.g. Oracle).
    ``conn`` carries optional host/port/socket/user/password."""
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

    argv, stdout_path = _native_command(spec["kind"], out_dir, conn)
    if not argv:
        log(f"  DB {name}: no adapter")
        return False
    env = dict(os.environ)
    pw = (conn or {}).get("password")
    if pw:  # safe password passing for the tools that read it from the env
        env.setdefault("PGPASSWORD", pw)
        env.setdefault("MYSQL_PWD", pw)
    try:
        if stdout_path:
            with open(stdout_path, "w", encoding="utf-8") as f:
                p = subprocess.run(argv, stdout=f, stderr=subprocess.PIPE, env=env)
        else:
            p = subprocess.run(argv, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, env=env)
    except OSError as e:
        log(f"  DB {name}: error {e}")
        return False
    if p.returncode == 0:
        log(f"  DB {name}: ok")
        return True
    err = p.stderr.decode("utf-8", "replace").strip().splitlines()
    log(f"  DB {name}: FAILED — {err[-1] if err else 'exit ' + str(p.returncode)}")
    return False


def dump_all(specs: list, out_dir: str, log=print, conn: dict = None) -> int:
    """Dump every spec into out_dir; return the number that succeeded."""
    ok = 0
    for spec in specs:
        if dump_database(spec, out_dir, log, conn):
            ok += 1
    return ok
