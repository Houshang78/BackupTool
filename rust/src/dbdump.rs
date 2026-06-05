// SPDX-License-Identifier: GPL-3.0-or-later
//! Database dump adapters. Detect locally installed databases and dump them into
//! the backup set's `.backuptool-db/` folder. Native adapters (run as root via
//! the local socket): PostgreSQL, MySQL/MariaDB, Redis, MongoDB. Anything else
//! (Oracle, MS SQL, ...) uses a *generic* adapter: a user-supplied shell command
//! that runs with `BACKUPTOOL_DB_OUT` pointing at the output folder.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const DB_DIR: &str = ".backuptool-db";

#[derive(Clone, Debug)]
pub struct DbSpec {
    pub name: String,
    pub kind: String,          // postgresql|mysql|redis|mongodb|generic
    pub shell: Option<String>, // generic command (Oracle, MS SQL, ...)
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
    let mut v = Vec::new();
    let mut add = |name: &str, kind: &str| v.push(DbSpec { name: name.into(), kind: kind.into(), shell: None });
    if has("pg_dumpall") {
        add("postgresql", "postgresql");
    }
    if has("mysqldump") || has("mariadb-dump") {
        add("mysql", "mysql");
    }
    if has("redis-cli") {
        add("redis", "redis");
    }
    if has("mongodump") {
        add("mongodb", "mongodb");
    }
    v
}

fn native(kind: &str, out_dir: &Path) -> Option<(Vec<String>, Option<PathBuf>)> {
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
            Some((argv, Some(out_dir.join("postgresql-all.sql"))))
        }
        "mysql" => {
            let dump = if has("mariadb-dump") { "mariadb-dump" } else { "mysqldump" };
            Some((
                vec![dump.into(), "--all-databases".into(), "--single-transaction".into(),
                     "--routines".into(), "--events".into()],
                Some(out_dir.join("mysql-all.sql")),
            ))
        }
        "redis" => Some((
            vec!["redis-cli".into(), "--rdb".into(), out_dir.join("redis-dump.rdb").to_string_lossy().into_owned()],
            None,
        )),
        "mongodb" => Some((
            vec!["mongodump".into(), "--out".into(), out_dir.join("mongodb").to_string_lossy().into_owned()],
            None,
        )),
        _ => None,
    }
}

pub fn dump_database<L: Fn(&str)>(spec: &DbSpec, out_dir: &Path, log: &L) -> bool {
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
        let (argv, stdout_path) = match native(&spec.kind, out_dir) {
            Some(x) => x,
            None => {
                log(&format!("  DB {}: no adapter", spec.name));
                return false;
            }
        };
        let mut c = Command::new(&argv[0]);
        c.args(&argv[1..]);
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

pub fn dump_all<L: Fn(&str)>(specs: &[DbSpec], out_dir: &Path, log: &L) -> usize {
    specs.iter().filter(|s| dump_database(s, out_dir, log)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_dump_writes_output() {
        let dir = std::env::temp_dir().join("bt-dbdump-test");
        let _ = std::fs::remove_dir_all(&dir);
        let spec = DbSpec {
            name: "demo".into(),
            kind: "generic".into(),
            shell: Some("printf data > \"$BACKUPTOOL_DB_OUT/demo.txt\"".into()),
        };
        let ok = dump_database(&spec, &dir, &|_m: &str| {});
        assert!(ok);
        assert!(dir.join("demo.txt").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn generic_dump_failure_reported() {
        let dir = std::env::temp_dir().join("bt-dbdump-fail");
        let spec = DbSpec { name: "boom".into(), kind: "generic".into(), shell: Some("exit 3".into()) };
        assert!(!dump_database(&spec, &dir, &|_m: &str| {}));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
