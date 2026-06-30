// SPDX-License-Identifier: GPL-3.0-or-later
//! Database dump adapters. Detect locally installed databases and dump them into
//! the backup set's `.backuptool-db/` folder. Native adapters (run as root via
//! the local socket): PostgreSQL, MySQL/MariaDB, Redis, MongoDB. Anything else
//! (Oracle, MS SQL, ...) uses a *generic* adapter: a user-supplied shell command
//! that runs with `BACKUPTOOL_DB_OUT` pointing at the output folder.

use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub const DB_DIR: &str = ".backuptool-db";

#[derive(Clone, Debug)]
pub struct DbSpec {
    pub name: String,
    pub kind: String,          // postgresql|mysql|redis|mongodb|generic
    pub shell: Option<String>, // generic command (Oracle, MS SQL, ...)
    pub running: bool,
}

/// Optional connection settings applied to the native dump commands.
#[derive(Clone, Default, Debug)]
pub struct DbConn {
    pub host: Option<String>,
    pub port: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub socket: Option<String>,
}

fn default_port(kind: &str) -> Option<u16> {
    match kind {
        "postgresql" => Some(5432),
        "mysql" => Some(3306),
        "redis" => Some(6379),
        "mongodb" => Some(27017),
        _ => None,
    }
}

fn default_sockets(kind: &str) -> &'static [&'static str] {
    match kind {
        "postgresql" => &["/var/run/postgresql/.s.PGSQL.5432", "/tmp/.s.PGSQL.5432"],
        "mysql" => &["/run/mysqld/mysqld.sock", "/var/run/mysqld/mysqld.sock", "/tmp/mysql.sock"],
        "redis" => &["/var/run/redis/redis-server.sock", "/run/redis/redis-server.sock"],
        "mongodb" => &["/tmp/mongodb-27017.sock"],
        _ => &[],
    }
}

/// Best-effort check that the database accepts connections (TCP probe, then a
/// default/explicit unix socket file).
pub fn is_running(kind: &str, conn: &DbConn) -> bool {
    let host = conn.host.clone().unwrap_or_else(|| "127.0.0.1".into());
    let port = conn.port.as_ref().and_then(|p| p.parse::<u16>().ok()).or_else(|| default_port(kind));
    if let Some(p) = port {
        if let Ok(addrs) = format!("{host}:{p}").to_socket_addrs() {
            for a in addrs {
                if std::net::TcpStream::connect_timeout(&a, Duration::from_millis(400)).is_ok() {
                    return true;
                }
            }
        }
    }
    let mut socks: Vec<String> = default_sockets(kind).iter().map(|s| s.to_string()).collect();
    if let Some(s) = &conn.socket {
        socks.insert(0, s.clone());
    }
    socks.iter().any(|s| Path::new(s).exists())
}

fn has(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|p| p.join(bin).is_file()))
        .unwrap_or(false)
}

#[cfg(unix)]
fn am_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}
#[cfg(not(unix))]
fn am_root() -> bool {
    false
}

pub fn detect_databases() -> Vec<DbSpec> {
    let mk = |name: &str, kind: &str| DbSpec { name: name.into(), kind: kind.into(), shell: None, running: false };
    let mut v = Vec::new();
    if has("pg_dumpall") {
        v.push(mk("postgresql", "postgresql"));
    }
    if has("mysqldump") || has("mariadb-dump") {
        v.push(mk("mysql", "mysql"));
    }
    if has("redis-cli") {
        v.push(mk("redis", "redis"));
    }
    if has("mongodump") {
        v.push(mk("mongodb", "mongodb"));
    }
    for d in v.iter_mut() {
        d.running = is_running(&d.kind, &DbConn::default());
    }
    v
}

fn native(kind: &str, out_dir: &Path, conn: &DbConn) -> Option<(Vec<String>, Option<PathBuf>)> {
    let (host, port, user, sock, pw) = (
        conn.host.as_deref(), conn.port.as_deref(), conn.user.as_deref(),
        conn.socket.as_deref(), conn.password.as_deref(),
    );
    let push = |argv: &mut Vec<String>, flag: &str, val: Option<&str>| {
        if let Some(v) = val {
            argv.push(flag.into());
            argv.push(v.into());
        }
    };
    match kind {
        "postgresql" => {
            let mut argv: Vec<String> = if am_root() {
                vec!["runuser".into(), "-u".into(), "postgres".into(), "--".into()]
            } else if has("sudo") {
                vec!["sudo".into(), "-n".into(), "-u".into(), "postgres".into()]
            } else {
                vec![]
            };
            argv.push("pg_dumpall".into());
            push(&mut argv, "-h", host);
            push(&mut argv, "-p", port);
            push(&mut argv, "-U", user);
            Some((argv, Some(out_dir.join("postgresql-all.sql"))))
        }
        "mysql" => {
            let dump = if has("mariadb-dump") { "mariadb-dump" } else { "mysqldump" };
            let mut argv = vec![dump.to_string(), "--all-databases".into(),
                                "--single-transaction".into(), "--routines".into(), "--events".into()];
            push(&mut argv, "-h", host);
            push(&mut argv, "-P", port);
            push(&mut argv, "-u", user);
            push(&mut argv, "--socket", sock);
            Some((argv, Some(out_dir.join("mysql-all.sql"))))
        }
        "redis" => {
            let mut argv = vec!["redis-cli".to_string()];
            push(&mut argv, "-h", host);
            push(&mut argv, "-p", port);
            push(&mut argv, "-s", sock);
            push(&mut argv, "-a", pw);
            argv.push("--rdb".into());
            argv.push(out_dir.join("redis-dump.rdb").to_string_lossy().into_owned());
            Some((argv, None))
        }
        "mongodb" => {
            let mut argv = vec!["mongodump".to_string()];
            push(&mut argv, "--host", host);
            push(&mut argv, "--port", port);
            push(&mut argv, "-u", user);
            push(&mut argv, "-p", pw);
            argv.push("--out".into());
            argv.push(out_dir.join("mongodb").to_string_lossy().into_owned());
            Some((argv, None))
        }
        _ => None,
    }
}

pub fn dump_database<L: Fn(&str)>(spec: &DbSpec, out_dir: &Path, conn: &DbConn, log: &L) -> bool {
    let _ = std::fs::create_dir_all(out_dir);
    let err_path = out_dir.join(format!("{}.err", spec.name));
    let err_file = || std::fs::File::create(&err_path).ok();

    let status = if let Some(cmd) = &spec.shell {
        let mut c = Command::new("sh");
        c.arg("-c").arg(cmd).env("BACKUPTOOL_DB_OUT", out_dir);
        if let Ok(f) = std::fs::File::create(out_dir.join(format!("{}.log", spec.name))) {
            c.stdout(Stdio::from(f));
        }
        if let Some(f) = err_file() {
            c.stderr(Stdio::from(f));
        }
        c.status()
    } else {
        let (argv, stdout_path) = match native(&spec.kind, out_dir, conn) {
            Some(x) => x,
            None => {
                log(&format!("  DB {}: no adapter", spec.name));
                return false;
            }
        };
        let mut c = Command::new(&argv[0]);
        c.args(&argv[1..]);
        if let Some(pw) = &conn.password {   // safe password passing for pg/mysql
            c.env("PGPASSWORD", pw);
            c.env("MYSQL_PWD", pw);
        }
        match &stdout_path {
            Some(p) => {
                if let Ok(f) = std::fs::File::create(p) {
                    c.stdout(Stdio::from(f));
                }
            }
            None => {
                c.stdout(Stdio::null());
            }
        }
        if let Some(f) = err_file() {
            c.stderr(Stdio::from(f));
        }
        c.status()
    };

    match status {
        Ok(s) if s.success() => {
            let _ = std::fs::remove_file(&err_path);
            log(&format!("  DB {}: ok", spec.name));
            true
        }
        Ok(s) => {
            log(&format!("  DB {}: FAILED ({s}); see {}", spec.name, err_path.display()));
            false
        }
        Err(e) => {
            log(&format!("  DB {}: error {e}", spec.name));
            false
        }
    }
}

pub fn dump_all<L: Fn(&str)>(specs: &[DbSpec], out_dir: &Path, conn: &DbConn, log: &L) -> usize {
    specs.iter().filter(|s| dump_database(s, out_dir, conn, log)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]  // the generic adapter runs a POSIX shell command (no sh/printf on Windows)
    #[test]
    fn generic_dump_writes_output() {
        let dir = std::env::temp_dir().join("bt-dbdump-test");
        let _ = std::fs::remove_dir_all(&dir);
        let spec = DbSpec {
            name: "demo".into(),
            kind: "generic".into(),
            shell: Some("printf data > \"$BACKUPTOOL_DB_OUT/demo.txt\"".into()),
            running: false,
        };
        let ok = dump_database(&spec, &dir, &DbConn::default(), &|_m: &str| {});
        assert!(ok);
        assert!(dir.join("demo.txt").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]  // POSIX shell command; on Windows there is no sh to run it
    #[test]
    fn generic_dump_failure_reported() {
        let dir = std::env::temp_dir().join("bt-dbdump-fail");
        let spec = DbSpec { name: "boom".into(), kind: "generic".into(), shell: Some("exit 3".into()), running: false };
        assert!(!dump_database(&spec, &dir, &DbConn::default(), &|_m: &str| {}));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
