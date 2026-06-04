// SPDX-License-Identifier: GPL-3.0-or-later
//! Engine: scan, incremental diff, parallel copy/encrypt, restore.

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use globset::{Glob, GlobSet, GlobSetBuilder};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use walkdir::WalkDir;

use crate::crypto::{self, Cipher};
use crate::manifest::{self, FileMeta, KdfParams, Kind, Manifest, MANIFEST_NAME};

pub const DEFAULT_EXCLUDES: &[&str] = &[
    "**/.cache/**", "**/.local/share/Trash/**", "**/.thumbnails/**",
    "**/Cache/**", "**/cache2/**", "**/lost+found",
    "**/.Spotlight-V100/**", "**/.Trashes/**", "**/.fseventsd/**", "**/.DS_Store",
];

pub struct Entry {
    pub rel: String,
    pub abspath: PathBuf,
    pub meta: FileMeta,
}

#[derive(Default, Debug)]
pub struct Stats {
    pub copied: u64,
    pub skipped: u64,
    pub bytes: u64,
    pub deleted: u64,
    pub errors: u64,
}

pub struct BackupOptions {
    pub sources: Vec<String>,
    pub dest: String,
    pub set: String,
    pub workers: usize,
    pub use_checksum: bool,
    pub excludes: Vec<String>,
    pub prune: bool,
    pub dry_run: bool,
    pub include_system: bool,
    pub cipher: Cipher,
    pub passphrase: Option<String>,
}

pub struct RestoreOptions {
    pub backup_dir: String,
    pub set: Option<String>,
    pub target: String,
    pub workers: usize,
    pub reapply_meta: bool,
    pub dry_run: bool,
    pub passphrase: Option<String>,
}

// ------------------------------------------------------------- platform
#[cfg(unix)]
fn meta_of(path: &Path) -> Option<FileMeta> {
    use std::os::unix::fs::MetadataExt;
    let md = std::fs::symlink_metadata(path).ok()?;
    let ft = md.file_type();
    let (kind, target) = if ft.is_symlink() {
        (Kind::Symlink, std::fs::read_link(path).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default())
    } else if ft.is_file() {
        (Kind::File, String::new())
    } else {
        return None;
    };
    Some(FileMeta {
        size: md.len(), mtime: md.mtime(), mode: md.mode(),
        uid: md.uid(), gid: md.gid(), kind, target, blake3: None,
    })
}

#[cfg(not(unix))]
fn meta_of(path: &Path) -> Option<FileMeta> {
    let md = std::fs::symlink_metadata(path).ok()?;
    let ft = md.file_type();
    let (kind, target) = if ft.is_symlink() {
        (Kind::Symlink, std::fs::read_link(path).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default())
    } else if ft.is_file() {
        (Kind::File, String::new())
    } else {
        return None;
    };
    Some(FileMeta {
        size: md.len(), mtime: 0, mode: 0o644, uid: 0, gid: 0,
        kind, target, blake3: None,
    })
}

fn build_globset(extra: &[String]) -> Result<GlobSet> {
    let mut b = GlobSetBuilder::new();
    for g in DEFAULT_EXCLUDES.iter().map(|s| s.to_string()).chain(extra.iter().cloned()) {
        b.add(Glob::new(&g).map_err(|e| anyhow!("exclude pattern '{g}': {e}"))?);
    }
    Ok(b.build()?)
}

/// Curated system directories worth backing up (e.g. /etc). Returns the ones that
/// exist — best read as root. Unreadable entries are simply skipped during scan.
pub fn system_dirs() -> Vec<String> {
    let cands: &[&str] = if cfg!(target_os = "macos") {
        &["/etc", "/usr/local/etc", "/opt"]
    } else if cfg!(target_os = "linux") {
        &["/etc", "/usr/local/etc", "/opt", "/srv", "/root", "/var/spool/cron"]
    } else {
        &[]
    };
    cands.iter()
        .filter(|d| Path::new(d).is_dir())
        .map(|s| s.to_string())
        .collect()
}

fn hash_file(path: &Path) -> Option<String> {
    let mut hasher = blake3::Hasher::new();
    let mut f = std::fs::File::open(path).ok()?;
    std::io::copy(&mut f, &mut hasher).ok()?;
    Some(hasher.finalize().to_hex().to_string())
}

pub fn scan(sources: &[String], excl: &GlobSet) -> Vec<Entry> {
    let mut out = Vec::new();
    for src in sources {
        let base = std::fs::canonicalize(src).unwrap_or_else(|_| PathBuf::from(src));
        for de in WalkDir::new(&base).follow_links(false).into_iter().filter_map(|e| e.ok()) {
            let ft = de.file_type();
            if !ft.is_file() && !ft.is_symlink() {
                continue;
            }
            let p = de.path();
            if excl.is_match(p) {
                continue;
            }
            if let Some(meta) = meta_of(p) {
                let rel = p.to_string_lossy().trim_start_matches('/').to_string();
                out.push(Entry { rel, abspath: p.to_path_buf(), meta });
            }
        }
    }
    out
}

fn needs_copy(meta: &FileMeta, prev: Option<&FileMeta>, use_checksum: bool) -> bool {
    match prev {
        None => true,
        Some(p) => {
            if meta.kind != p.kind {
                return true;
            }
            if meta.kind == Kind::Symlink {
                return meta.target != p.target;
            }
            if use_checksum {
                return meta.blake3 != p.blake3;
            }
            meta.size != p.size || meta.mtime != p.mtime
        }
    }
}

fn copy_one(e: &Entry, set_root: &Path, cipher: Cipher, key: Option<&[u8; 32]>) -> Result<u64> {
    let dst = set_root.join(&e.rel);
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if e.meta.kind == Kind::Symlink {
        let _ = std::fs::remove_file(&dst);
        #[cfg(unix)]
        {
            // Destination FS without symlinks (exFAT) fails -> info stays in manifest.
            let _ = std::os::unix::fs::symlink(&e.meta.target, &dst);
        }
        return Ok(0);
    }
    match cipher {
        Cipher::None => {
            std::fs::copy(&e.abspath, &dst)?;
        }
        _ => {
            let data = std::fs::read(&e.abspath)?;
            let blob = crypto::encrypt(cipher, key.ok_or_else(|| anyhow!("missing key"))?, &data)?;
            std::fs::write(&dst, blob)?;
        }
    }
    Ok(e.meta.size)
}

pub fn backup<P, L>(opt: &BackupOptions, progress: P, log: L) -> Result<Stats>
where
    P: Fn(u64, u64) + Sync + Send,
    L: Fn(&str) + Sync + Send,
{
    let excl = build_globset(&opt.excludes)?;
    let set_root = Path::new(&opt.dest).join(&opt.set);
    let manifest_path = set_root.join(MANIFEST_NAME);
    let prev = manifest::load(&manifest_path);
    let prev_files: BTreeMap<String, FileMeta> =
        prev.as_ref().map(|m| m.files.clone()).unwrap_or_default();

    let mut sources = opt.sources.clone();
    if opt.include_system {
        let sd = system_dirs();
        log(&format!("Including system directories: {}",
            if sd.is_empty() { "(none)".to_string() } else { sd.join(", ") }));
        sources.extend(sd);
    }

    log("Scanning sources ...");
    let mut entries = scan(&sources, &excl);
    log(&format!("{} files found.", entries.len()));

    if opt.use_checksum {
        log("Computing BLAKE3 (parallel across all cores) ...");
        entries.par_iter_mut().for_each(|e| {
            if e.meta.kind == Kind::File {
                e.meta.blake3 = hash_file(&e.abspath);
            }
        });
    }

    // Derive key (reuse the salt from the previous set, otherwise create a new one)
    let (kdf, key): (Option<KdfParams>, Option<[u8; 32]>) = if opt.cipher != Cipher::None {
        let pass = opt.passphrase.clone().ok_or_else(|| anyhow!("password missing"))?;
        let (salt, m, t, p) = match &prev {
            Some(pm) if pm.cipher == opt.cipher.name() && pm.kdf.is_some() => {
                let k = pm.kdf.as_ref().unwrap();
                (B64.decode(&k.salt_b64)?, k.m_cost, k.t_cost, k.p_cost)
            }
            _ => (
                crypto::random_salt().to_vec(),
                crypto::DEFAULT_M_COST,
                crypto::DEFAULT_T_COST,
                crypto::DEFAULT_P_COST,
            ),
        };
        let key = crypto::derive_key(&pass, &salt, m, t, p)?;
        let kdf = KdfParams {
            algo: "argon2id".into(),
            salt_b64: B64.encode(&salt),
            m_cost: m, t_cost: t, p_cost: p,
        };
        (Some(kdf), Some(key))
    } else {
        (None, None)
    };

    let todo: Vec<&Entry> = entries
        .iter()
        .filter(|e| needs_copy(&e.meta, prev_files.get(&e.rel), opt.use_checksum))
        .collect();
    let unchanged = (entries.len() - todo.len()) as u64;
    let current: HashSet<&String> = entries.iter().map(|e| &e.rel).collect();
    let deletions: Vec<String> = prev_files.keys().filter(|k| !current.contains(k)).cloned().collect();
    log(&format!(
        "Changed/new: {} | unchanged: {} | deleted in source: {}",
        todo.len(), unchanged, deletions.len()
    ));

    if opt.dry_run {
        for e in todo.iter().take(1000) {
            log(&format!("  [would back up] /{}", e.rel));
        }
        return Ok(Stats { skipped: unchanged, ..Default::default() });
    }

    std::fs::create_dir_all(&set_root)?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(opt.workers.max(1))
        .build()?;
    let done = AtomicU64::new(0);
    let copied = AtomicU64::new(0);
    let errors = AtomicU64::new(0);
    let bytes = AtomicU64::new(0);
    let total = todo.len() as u64;

    pool.install(|| {
        todo.par_iter().for_each(|e| {
            match copy_one(e, &set_root, opt.cipher, key.as_ref()) {
                Ok(n) => {
                    copied.fetch_add(1, Ordering::Relaxed);
                    bytes.fetch_add(n, Ordering::Relaxed);
                }
                Err(err) => {
                    errors.fetch_add(1, Ordering::Relaxed);
                    log(&format!("  ERROR {}: {}", e.rel, err));
                }
            }
            let d = done.fetch_add(1, Ordering::Relaxed) + 1;
            progress(d, total);
        });
    });

    // Mirror deletions
    let mut deleted = 0u64;
    if opt.prune {
        for rel in &deletions {
            let tgt = set_root.join(rel);
            if std::fs::remove_file(&tgt).is_ok() {
                deleted += 1;
            }
        }
    }

    // Write manifest
    let mut files = BTreeMap::new();
    for e in &entries {
        files.insert(e.rel.clone(), e.meta.clone());
    }
    let man = Manifest {
        version: 1,
        set: opt.set.clone(),
        host: gethostname::gethostname().to_string_lossy().into_owned(),
        created: chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
        use_checksum: opt.use_checksum,
        cipher: opt.cipher.name().to_string(),
        kdf,
        files,
    };
    manifest::save(&manifest_path, &man)?;

    let stats = Stats {
        copied: copied.load(Ordering::Relaxed),
        skipped: unchanged,
        bytes: bytes.load(Ordering::Relaxed),
        deleted,
        errors: errors.load(Ordering::Relaxed),
    };

    // Per-run, dated log listing exactly which full paths changed/were removed.
    let logdir = set_root.join(".backuptool-logs");
    if std::fs::create_dir_all(&logdir).is_ok() {
        let stamp = man.created.replace([':', '-'], "").replace('T', "-");
        let logpath = logdir.join(format!("backup-{stamp}.log"));
        let mut s = String::new();
        s.push_str(&format!("# backuptool  set={}  host={}  {}\n", man.set, man.host, man.created));
        s.push_str(&format!("# changed/new={} unchanged={} deleted={} errors={}\n",
            stats.copied, stats.skipped, stats.deleted, stats.errors));
        let mut changed: Vec<&str> = todo.iter().map(|e| e.rel.as_str()).collect();
        changed.sort_unstable();
        for r in changed {
            s.push_str(&format!("CHANGED\t/{r}\n"));
        }
        let mut dels = deletions.clone();
        dels.sort_unstable();
        for r in &dels {
            s.push_str(&format!("DELETED\t/{r}\n"));
        }
        if std::fs::write(&logpath, s).is_ok() {
            log(&format!("Log written: {}", logpath.display()));
        }
    }

    log(&format!(
        "Done: {} copied, {} skipped, {} removed, {} errors.",
        stats.copied, stats.skipped, stats.deleted, stats.errors
    ));
    Ok(stats)
}

#[cfg(unix)]
fn apply_meta(dst: &Path, m: &FileMeta) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dst, std::fs::Permissions::from_mode(m.mode & 0o7777));
    // chown fails with EPERM without root -> ignore
    let _ = std::os::unix::fs::chown(dst, Some(m.uid), Some(m.gid));
    let _ = filetime::set_file_mtime(dst, filetime::FileTime::from_unix_time(m.mtime, 0));
}

#[cfg(not(unix))]
fn apply_meta(dst: &Path, m: &FileMeta) {
    let _ = filetime::set_file_mtime(dst, filetime::FileTime::from_unix_time(m.mtime, 0));
}

pub fn restore<P, L>(opt: &RestoreOptions, progress: P, log: L) -> Result<Stats>
where
    P: Fn(u64, u64) + Sync + Send,
    L: Fn(&str) + Sync + Send,
{
    let set_root = match &opt.set {
        Some(s) => Path::new(&opt.backup_dir).join(s),
        None => PathBuf::from(&opt.backup_dir),
    };
    let manifest_path = set_root.join(MANIFEST_NAME);
    let man = manifest::load(&manifest_path)
        .ok_or_else(|| anyhow!("No manifest at {}", manifest_path.display()))?;

    let cipher = Cipher::parse(&man.cipher)?;
    let key: Option<[u8; 32]> = if cipher != Cipher::None {
        let k = man.kdf.as_ref().ok_or_else(|| anyhow!("KDF parameters missing"))?;
        let pass = opt.passphrase.clone().ok_or_else(|| anyhow!("password missing"))?;
        let salt = B64.decode(&k.salt_b64)?;
        Some(crypto::derive_key(&pass, &salt, k.m_cost, k.t_cost, k.p_cost)?)
    } else {
        None
    };

    let target = PathBuf::from(&opt.target);
    let items: Vec<(String, FileMeta)> = man.files.into_iter().collect();
    let total = items.len() as u64;
    log(&format!("Restoring {} entries -> {}", total, target.display()));

    if opt.dry_run {
        for (rel, _) in items.iter().take(1000) {
            log(&format!("  [would restore] {}", target.join(rel).display()));
        }
        return Ok(Stats::default());
    }

    let pool = rayon::ThreadPoolBuilder::new().num_threads(opt.workers.max(1)).build()?;
    let done = AtomicU64::new(0);
    let restored = AtomicU64::new(0);
    let errors = AtomicU64::new(0);

    pool.install(|| {
        items.par_iter().for_each(|(rel, meta)| {
            let res = (|| -> Result<()> {
                let src = set_root.join(rel);
                let dst = target.join(rel);
                if let Some(parent) = dst.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if meta.kind == Kind::Symlink {
                    let _ = std::fs::remove_file(&dst);
                    #[cfg(unix)]
                    {
                        std::os::unix::fs::symlink(&meta.target, &dst)?;
                    }
                    return Ok(());
                }
                match cipher {
                    Cipher::None => {
                        std::fs::copy(&src, &dst)?;
                    }
                    _ => {
                        let blob = std::fs::read(&src)?;
                        let data = crypto::decrypt(cipher, key.as_ref().unwrap(), &blob)?;
                        std::fs::write(&dst, data)?;
                    }
                }
                if opt.reapply_meta {
                    apply_meta(&dst, meta);
                }
                Ok(())
            })();
            match res {
                Ok(()) => { restored.fetch_add(1, Ordering::Relaxed); }
                Err(e) => { errors.fetch_add(1, Ordering::Relaxed); log(&format!("  ERROR {}: {}", rel, e)); }
            }
            let d = done.fetch_add(1, Ordering::Relaxed) + 1;
            progress(d, total);
        });
    });

    let stats = Stats {
        copied: restored.load(Ordering::Relaxed),
        errors: errors.load(Ordering::Relaxed),
        ..Default::default()
    };
    log(&format!("Restore done: {} restored, {} errors.", stats.copied, stats.errors));
    Ok(stats)
}

pub fn list_sets(dest: &str) -> Vec<(String, String, String, usize)> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dest) {
        for e in rd.flatten() {
            let mp = e.path().join(MANIFEST_NAME);
            if let Some(m) = manifest::load(&mp) {
                out.push((m.set, m.host, m.created, m.files.len()));
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fm(size: u64, mtime: i64) -> FileMeta {
        FileMeta {
            size, mtime, mode: 0o644, uid: 0, gid: 0,
            kind: Kind::File, target: String::new(), blake3: None,
        }
    }

    #[test]
    fn new_file_needs_copy() {
        assert!(needs_copy(&fm(1, 1), None, false));
    }

    #[test]
    fn unchanged_is_skipped() {
        assert!(!needs_copy(&fm(10, 100), Some(&fm(10, 100)), false));
    }

    #[test]
    fn changed_size_or_mtime_copies() {
        assert!(needs_copy(&fm(11, 100), Some(&fm(10, 100)), false));
        assert!(needs_copy(&fm(10, 101), Some(&fm(10, 100)), false));
    }

    #[test]
    fn system_dirs_all_exist() {
        for d in system_dirs() {
            assert!(std::path::Path::new(&d).is_dir());
        }
    }

    #[test]
    fn checksum_mode_compares_hash() {
        let mut a = fm(10, 100);
        a.blake3 = Some("aaa".into());
        let mut b = fm(10, 100);
        b.blake3 = Some("bbb".into());
        assert!(needs_copy(&a, Some(&b), true));
        b.blake3 = Some("aaa".into());
        assert!(!needs_copy(&a, Some(&b), true));
    }
}
