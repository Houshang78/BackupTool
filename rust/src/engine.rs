// SPDX-License-Identifier: GPL-3.0-or-later
//! Engine: scan, incremental diff, parallel copy/encrypt, restore.

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use walkdir::WalkDir;

use crate::crypto::{self, Cipher};
use crate::manifest::{self, FileMeta, KdfParams, Kind, Manifest, MANIFEST_NAME};

pub const DEFAULT_EXCLUDES: &[&str] = &[
    "**/.cache/**", "**/.local/share/Trash/**", "**/.thumbnails/**",
    "**/Cache/**", "**/cache2/**", "**/lost+found",
    "**/.Spotlight-V100/**", "**/.Trashes/**", "**/.fseventsd/**", "**/.DS_Store",
    // Volatile / always-locked junk (mainly Windows): browser caches, temp,
    // and the unreadable WindowsApps app-execution aliases.
    "**/Code Cache/**", "**/GPUCache/**", "**/DawnGraphiteCache/**", "**/DawnWebGPUCache/**",
    "**/AppData/Local/Temp/**", "**/AppData/Local/Microsoft/WindowsApps/**",
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
    /// VSS path remap: (shadow-device prefix, original-volume prefix). When set,
    /// files are read from the shadow copy but recorded under the original path.
    pub vss_map: Vec<(String, String)>,
    /// after copying, read the destination back and confirm files are really there.
    pub verify: bool,
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

/// Destination-relative path for a source file. Strips the leading '/' (unix)
/// or the drive / `\\?\` verbatim prefix (Windows) so the file lands UNDER the
/// destination instead of escaping it via an absolute join. A drive like `D:\`
/// becomes a top-level folder `D` (e.g. `\\?\D:\a\b` -> `D/a/b`).
fn to_rel(path: &str) -> String {
    let s = path.strip_prefix(r"\\?\").unwrap_or(path);
    let b = s.as_bytes();
    let s = if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        format!("{}{}", &s[..1], &s[2..]) // "C:\Users" -> "C\Users"
    } else {
        s.to_string()
    };
    s.trim_start_matches(['/', '\\']).to_string()
}

fn build_globset(extra: &[String]) -> Result<GlobSet> {
    let mut b = GlobSetBuilder::new();
    // Windows paths are case-insensitive; match excludes accordingly there.
    let ci = cfg!(windows);
    for g in DEFAULT_EXCLUDES.iter().map(|s| s.to_string()).chain(extra.iter().cloned()) {
        let glob = GlobBuilder::new(&g)
            .case_insensitive(ci)
            .build()
            .map_err(|e| anyhow!("exclude pattern '{g}': {e}"))?;
        b.add(glob);
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
        // filter_entry prunes excluded directories so we never descend into them
        // (skips Windows/, Program Files/, … entirely instead of walking them).
        let walk = WalkDir::new(&base)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !excl.is_match(e.path()));
        for de in walk.filter_map(|e| e.ok()) {
            let ft = de.file_type();
            if !ft.is_file() && !ft.is_symlink() {
                continue;
            }
            let p = de.path();
            if excl.is_match(p) {
                continue;
            }
            if let Some(meta) = meta_of(p) {
                let rel = to_rel(&p.to_string_lossy());
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
    // Selecting a whole volume root (C:\, /) auto-excludes the OS/program dirs,
    // so "back up C:" copies user data without drowning in Windows system files.
    let mut excludes = opt.excludes.clone();
    if !opt.include_system && opt.sources.iter().any(|s| is_volume_root(s)) {
        excludes.extend(disk_excludes());
        log("Volume root selected — auto-excluding system/OS directories (Windows, Program Files, …).");
    }
    let excl = build_globset(&excludes)?;
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
    // VSS: read from the shadow copy but record files under their original path.
    if !opt.vss_map.is_empty() {
        for e in entries.iter_mut() {
            let ap = e.abspath.to_string_lossy().to_string();
            for (shadow, orig) in &opt.vss_map {
                if let Some(rest) = ap.strip_prefix(shadow.as_str()) {
                    e.rel = to_rel(&format!("{orig}{rest}"));
                    break;
                }
            }
        }
    }
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
    // Error buckets for the end-of-run summary.
    let err_in_use = AtomicU64::new(0);
    let err_denied = AtomicU64::new(0);
    let err_notfound = AtomicU64::new(0);
    let err_other = AtomicU64::new(0);
    let total = todo.len() as u64;

    pool.install(|| {
        todo.par_iter().for_each(|e| {
            match copy_one(e, &set_root, opt.cipher, key.as_ref()) {
                Ok(n) => {
                    copied.fetch_add(1, Ordering::Relaxed);
                    bytes.fetch_add(n, Ordering::Relaxed);
                }
                Err(err) => {
                    let n = errors.fetch_add(1, Ordering::Relaxed);
                    let s = err.to_string();
                    if s.contains("(os error 32)") || s.contains("being used by another process") {
                        err_in_use.fetch_add(1, Ordering::Relaxed);
                    } else if s.contains("(os error 5)") || s.contains("(os error 13)")
                        || s.contains("Access is denied") || s.contains("denied") {
                        err_denied.fetch_add(1, Ordering::Relaxed);
                    } else if s.contains("(os error 2)") || s.contains("(os error 3)")
                        || s.contains("cannot find") || s.contains("No such file") {
                        err_notfound.fetch_add(1, Ordering::Relaxed);
                    } else {
                        err_other.fetch_add(1, Ordering::Relaxed);
                    }
                    if n < ERROR_LOG_CAP {
                        log(&format!("  ERROR {}: {}", e.rel, err));
                    } else if n == ERROR_LOG_CAP {
                        log(&format!("  ... further errors suppressed (over {ERROR_LOG_CAP}); see the summary below."));
                    }
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

    if stats.errors > 0 {
        log(&format!(
            "Errors by type: in-use(locked)={}, access-denied={}, not-found={}, other={}.",
            err_in_use.load(Ordering::Relaxed),
            err_denied.load(Ordering::Relaxed),
            err_notfound.load(Ordering::Relaxed),
            err_other.load(Ordering::Relaxed),
        ));
        log("Tip: 'in-use' files were open in another app — close it (or use clone/VSS) and re-run.");
    }
    log(&format!(
        "Done: {} copied, {} skipped, {} removed, {} errors.",
        stats.copied, stats.skipped, stats.deleted, stats.errors
    ));

    // Control: loud warning if a non-empty selection produced nothing usable.
    if !entries.is_empty() && stats.copied == 0 && stats.skipped == 0 {
        log("WARNING: 0 files reached the destination! Nothing was backed up — check the errors above and the destination path.");
    }

    // Control: read the destination back and confirm the files are really there.
    if opt.verify {
        log("Verifying destination (reading files back) ...");
        match verify_set(&set_root, opt.passphrase.as_deref(), &progress, &log) {
            Ok(v) => {
                if v.errors > 0 {
                    log(&format!("WARNING: verification found {} file(s) MISSING or corrupt at the destination!", v.errors));
                } else {
                    log(&format!("Verified OK: {} file(s) confirmed at the destination.", v.copied));
                }
            }
            Err(e) => log(&format!("Verification could not run: {e}")),
        }
    }
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

// ------------------------------------------------------------- verify

/// Re-read every file of a backup set from the destination, decrypt it when the
/// set is encrypted, and compare its BLAKE3 (or size, when no hash was recorded)
/// against the manifest. `Stats::copied` counts verified entries, `Stats::errors`
/// counts mismatches/unreadable files.
pub fn verify_set<P, L>(set_root: &Path, passphrase: Option<&str>, progress: P, log: L) -> Result<Stats>
where
    P: Fn(u64, u64) + Sync + Send,
    L: Fn(&str) + Sync + Send,
{
    let manifest_path = set_root.join(MANIFEST_NAME);
    let man = manifest::load(&manifest_path)
        .ok_or_else(|| anyhow!("No manifest at {}", manifest_path.display()))?;

    let cipher = Cipher::parse(&man.cipher)?;
    let key: Option<[u8; 32]> = if cipher != Cipher::None {
        let k = man.kdf.as_ref().ok_or_else(|| anyhow!("KDF parameters missing"))?;
        let pass = passphrase.ok_or_else(|| anyhow!("password missing"))?;
        let salt = B64.decode(&k.salt_b64)?;
        Some(crypto::derive_key(pass, &salt, k.m_cost, k.t_cost, k.p_cost)?)
    } else {
        None
    };

    let items: Vec<(String, FileMeta)> = man.files.into_iter().collect();
    let total = items.len() as u64;
    log(&format!("Verifying {total} entries ..."));

    let done = AtomicU64::new(0);
    let ok = AtomicU64::new(0);
    let errors = AtomicU64::new(0);

    items.par_iter().for_each(|(rel, meta)| {
        let res = (|| -> Result<()> {
            let src = set_root.join(rel);
            if meta.kind == Kind::Symlink {
                let got = std::fs::read_link(&src)
                    .map_err(|e| anyhow!("symlink unreadable: {e}"))?;
                if got.to_string_lossy() != meta.target {
                    return Err(anyhow!("symlink target mismatch"));
                }
                return Ok(());
            }
            let blob = std::fs::read(&src)?;
            let data = match cipher {
                Cipher::None => blob,
                _ => crypto::decrypt(cipher, key.as_ref().unwrap(), &blob)?,
            };
            match &meta.blake3 {
                Some(h) => {
                    let got = blake3::hash(&data).to_hex().to_string();
                    if &got != h {
                        return Err(anyhow!("BLAKE3 mismatch"));
                    }
                }
                None => {
                    if data.len() as u64 != meta.size {
                        return Err(anyhow!("size mismatch"));
                    }
                }
            }
            Ok(())
        })();
        match res {
            Ok(()) => { ok.fetch_add(1, Ordering::Relaxed); }
            Err(e) => { errors.fetch_add(1, Ordering::Relaxed); log(&format!("  VERIFY-FAIL {rel}: {e}")); }
        }
        let d = done.fetch_add(1, Ordering::Relaxed) + 1;
        progress(d, total);
    });

    let stats = Stats {
        copied: ok.load(Ordering::Relaxed),
        errors: errors.load(Ordering::Relaxed),
        ..Default::default()
    };
    log(&format!("Verify done: {} ok, {} failed.", stats.copied, stats.errors));
    Ok(stats)
}

// ------------------------------------------------------- evacuate (decommission)

pub struct EvacuateOptions {
    pub sources: Vec<String>,
    pub dest: String,
    pub set: String,
    pub workers: usize,
    pub excludes: Vec<String>,
    pub cipher: Cipher,
    pub passphrase: Option<String>,
    /// after a verified copy, delete the source files (turns the copy into a move)
    pub delete_source: bool,
    /// overwrite source file contents with random data before deleting (Phase 2)
    pub secure_wipe: bool,
    /// number of overwrite passes for `secure_wipe` (>=1)
    pub wipe_passes: u32,
    /// skip the "destination must be a different device" guard (testing/edge only)
    pub allow_same_device: bool,
    pub dry_run: bool,
}

#[derive(Default, Debug)]
pub struct EvacuateReport {
    pub scanned: u64,
    pub bytes: u64,
    pub copied: u64,
    pub backup_errors: u64,
    pub verified: u64,
    pub verify_errors: u64,
    pub deleted: u64,
    pub delete_errors: u64,
    /// of the deleted files, how many were securely overwritten first
    pub wiped: u64,
    /// detected storage type of the source when secure-wiping ("ssd"/"hdd"/"unknown")
    pub storage: String,
}

/// Resolve a scope keyword to concrete source paths.
/// `home` -> user data root, `config` -> the explicit paths, `disk` -> the whole
/// system root (or the explicit mount points), `auto` -> the current OS's
/// canonical user-data folders.
pub fn scope_sources(scope: &str, explicit: &[String]) -> Result<Vec<String>> {
    match scope {
        "config" => {
            if explicit.is_empty() {
                return Err(anyhow!("scope=config requires at least one source path"));
            }
            Ok(explicit.to_vec())
        }
        "home" => {
            let roots = home_roots();
            if roots.is_empty() {
                return Err(anyhow!("could not determine a home directory root on this platform; use scope=config"));
            }
            Ok(roots)
        }
        "disk" => {
            if explicit.is_empty() { Ok(vec![disk_root()]) } else { Ok(explicit.to_vec()) }
        }
        "auto" => {
            let a = auto_sources();
            if a.is_empty() {
                return Err(anyhow!("could not auto-detect user data folders; use scope=config"));
            }
            Ok(a)
        }
        other => Err(anyhow!("unknown scope '{other}' (use: home | config | disk | auto)")),
    }
}

/// The current user's canonical data folders for this OS (only ones that exist).
/// Used by the "Auto" button / `--auto` to fill in sources without the OS itself.
pub fn auto_sources() -> Vec<String> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok();
    let mut out = Vec::new();
    if let Some(h) = home {
        let h = PathBuf::from(h);
        let subs: &[&str] = if cfg!(windows) {
            &["Documents", "Desktop", "Downloads", "Pictures", "Music", "Videos",
              "Favorites", "Saved Games", "OneDrive", "AppData\\Roaming"]
        } else if cfg!(target_os = "macos") {
            &["Documents", "Desktop", "Downloads", "Pictures", "Music", "Movies",
              "Library/Application Support", "Library/Preferences", "Library/Mail"]
        } else {
            &["Documents", "Desktop", "Downloads", "Pictures", "Music", "Videos",
              ".config", ".local/share", ".mozilla", ".thunderbird", ".ssh", ".gnupg"]
        };
        for s in subs {
            let p = h.join(s);
            if p.exists() {
                out.push(p.to_string_lossy().into_owned());
            }
        }
        if out.is_empty() {
            out.push(h.to_string_lossy().into_owned());
        }
    }
    out
}

fn home_roots() -> Vec<String> {
    if cfg!(target_os = "macos") {
        vec!["/Users".into()]
    } else if cfg!(target_os = "linux") {
        vec!["/home".into()]
    } else if cfg!(windows) {
        match std::env::var("SystemDrive") {
            Ok(d) => vec![format!("{d}\\Users")],
            Err(_) => vec!["C:\\Users".into()],
        }
    } else {
        vec![]
    }
}

fn disk_root() -> String {
    if cfg!(windows) {
        std::env::var("SystemDrive").map(|d| format!("{d}\\")).unwrap_or_else(|_| "C:\\".into())
    } else {
        "/".into()
    }
}

/// The volume/mount root that `path` lives on (the top dir still on the same
/// device). On Windows, the drive root.
fn mount_root(path: &Path) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(m0) = std::fs::metadata(path) {
            let dev0 = m0.dev();
            let mut cur = path.to_path_buf();
            while let Some(parent) = cur.parent() {
                match std::fs::metadata(parent) {
                    Ok(m) if m.dev() == dev0 => cur = parent.to_path_buf(),
                    _ => break,
                }
            }
            return cur;
        }
    }
    #[cfg(windows)]
    {
        let s = path.to_string_lossy();
        if s.len() >= 2 && s.as_bytes()[1] == b':' {
            return PathBuf::from(format!("{}\\", &s[..2]));
        }
    }
    path.to_path_buf()
}

/// A suggested backup destination: the volume the running app lives on, plus a
/// dated, per-user folder (e.g. `/Volumes/Stick/backuptool-20260613-alice`).
/// Handy for portable use — run the tool from the backup drive and it proposes
/// that same drive as the target.
pub fn suggested_dest() -> String {
    let base = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let vol = mount_root(&base);
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".to_string());
    let date = chrono::Local::now().format("%Y%m%d").to_string();
    vol.join(format!("backuptool-{date}-{user}")).to_string_lossy().into_owned()
}

/// Whether `s` names a whole volume/drive root (`/`, `C:`, `C:\`, `\\?\C:\`).
fn is_volume_root(s: &str) -> bool {
    let t = s.trim();
    if t == "/" {
        return true;
    }
    let t = t.strip_prefix(r"\\?\").unwrap_or(t);
    let b = t.as_bytes();
    (b.len() == 2 && b[1] == b':')
        || (b.len() == 3 && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
}

/// Cap on individual error lines logged per run (the rest are summarized) so a
/// failing source can't produce a multi-hundred-MB log or flood the GUI thread.
const ERROR_LOG_CAP: u64 = 200;

/// Glob patterns protecting OS/system locations when evacuating a whole disk.
pub fn disk_excludes() -> Vec<String> {
    let pats: &[&str] = if cfg!(target_os = "macos") {
        &["/System/**", "/usr/**", "/bin/**", "/sbin/**", "/private/**", "/Library/**",
          "/Applications/**", "/Volumes/**", "/dev/**", "/cores/**", "/.vol/**", "/.fseventsd/**"]
    } else if cfg!(target_os = "linux") {
        &["/usr/**", "/bin/**", "/sbin/**", "/lib/**", "/lib64/**", "/boot/**", "/proc/**",
          "/sys/**", "/dev/**", "/run/**", "/var/**", "/etc/**", "/tmp/**", "/snap/**",
          "/mnt/**", "/media/**", "/lost+found"]
    } else if cfg!(windows) {
        // Exclude the OS and program directories; keep user data (C:\Users, etc.).
        // Patterns are anchored with **/ so they match regardless of drive letter
        // and the \\?\ prefix; each name covers both the folder and its contents.
        &["**/Windows", "**/Windows/**",
          "**/Program Files", "**/Program Files/**",
          "**/Program Files (x86)", "**/Program Files (x86)/**",
          "**/ProgramData", "**/ProgramData/**",
          "**/$Recycle.Bin", "**/$Recycle.Bin/**",
          "**/System Volume Information", "**/System Volume Information/**",
          "**/Recovery", "**/Recovery/**",
          "**/PerfLogs", "**/PerfLogs/**",
          "**/$WinREAgent", "**/$WinREAgent/**",
          "**/Windows.old", "**/Windows.old/**",
          "**/Config.Msi", "**/Config.Msi/**",
          "**/pagefile.sys", "**/hiberfil.sys", "**/swapfile.sys"]
    } else {
        &[]
    };
    pats.iter().map(|s| s.to_string()).collect()
}

#[cfg(unix)]
fn same_device(a: &Path, b: &Path) -> Option<bool> {
    use std::os::unix::fs::MetadataExt;
    let da = std::fs::metadata(a).ok()?.dev();
    let db = std::fs::metadata(b).ok()?.dev();
    Some(da == db)
}

#[cfg(not(unix))]
fn same_device(_a: &Path, _b: &Path) -> Option<bool> {
    None
}

#[cfg(unix)]
fn free_space(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    unsafe {
        let mut s: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut s) == 0 {
            Some(s.f_bavail as u64 * s.f_frsize as u64)
        } else {
            None
        }
    }
}

#[cfg(not(unix))]
fn free_space(_path: &Path) -> Option<u64> {
    None
}

/// A key identifying the volume/device a path lives on, so files can be grouped
/// per device when a machine has several (possibly mixed SSD/HDD) disks.
fn volume_key(path: &Path) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(md) = std::fs::metadata(path) {
            return md.dev().to_string();
        }
    }
    // Fallback (and Windows): group by the path prefix / drive component.
    path.components().next()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Overwrite a file's contents with random data (`passes` times), then truncate
/// it to zero length. The caller still unlinks the file afterwards.
/// Note: on SSDs and copy-on-write filesystems (APFS, Btrfs, ZFS) this does NOT
/// guarantee the old blocks are unrecoverable — the caller warns about that.
fn secure_overwrite(path: &Path, passes: u32) -> Result<()> {
    use rand::RngCore;
    use std::io::{Seek, SeekFrom, Write};
    let len = std::fs::metadata(path)?.len();
    let mut f = std::fs::OpenOptions::new().write(true).open(path)?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut rng = rand::rngs::OsRng;
    for _ in 0..passes.max(1) {
        f.seek(SeekFrom::Start(0))?;
        let mut remaining = len;
        while remaining > 0 {
            let n = remaining.min(buf.len() as u64) as usize;
            rng.fill_bytes(&mut buf[..n]);
            f.write_all(&buf[..n])?;
            remaining -= n as u64;
        }
        f.flush()?;
        f.sync_all()?;
    }
    f.set_len(0)?;
    f.sync_all()?;
    Ok(())
}

/// Evacuate (move) files to external storage: copy + verify, and only after a
/// fully verified copy optionally delete the sources. Nothing is ever deleted
/// if the backup or verification reports a single error.
pub fn evacuate<P, L>(opt: &EvacuateOptions, progress: P, log: L) -> Result<EvacuateReport>
where
    P: Fn(u64, u64) + Sync + Send,
    L: Fn(&str) + Sync + Send,
{
    let set_root = Path::new(&opt.dest).join(&opt.set);
    std::fs::create_dir_all(&opt.dest)?;

    // Pre-scan to know the source files (for size accounting and deletion).
    let excl = build_globset(&opt.excludes)?;
    let entries = scan(&opt.sources, &excl);
    let scanned = entries.len() as u64;
    let total_bytes: u64 = entries.iter()
        .filter(|e| e.meta.kind == Kind::File)
        .map(|e| e.meta.size)
        .sum();
    log(&format!("{scanned} files to evacuate ({:.1} MiB).", total_bytes as f64 / 1_048_576.0));

    // Safety: refuse to "move" onto the same physical device as a source.
    if !opt.allow_same_device {
        for s in &opt.sources {
            if same_device(Path::new(s), Path::new(&opt.dest)) == Some(true) {
                return Err(anyhow!(
                    "Refusing: destination '{}' is on the SAME device as source '{}'. \
                     Use an external disk/stick (or pass allow_same_device).",
                    opt.dest, s
                ));
            }
        }
    }

    // Safety: ensure the destination has room for the data.
    if let Some(free) = free_space(Path::new(&opt.dest)) {
        if free < total_bytes {
            return Err(anyhow!(
                "Not enough free space at '{}': need {} bytes, have {}.",
                opt.dest, total_bytes, free
            ));
        }
    }

    if opt.dry_run {
        log("[dry-run] would copy + verify the above; no source is deleted.");
        for e in entries.iter().take(1000) {
            log(&format!("  [would move] {}", e.abspath.display()));
        }
        return Ok(EvacuateReport { scanned, bytes: total_bytes, ..Default::default() });
    }

    // Copy phase — force checksum so the manifest carries hashes to verify against.
    let bopt = BackupOptions {
        sources: opt.sources.clone(),
        dest: opt.dest.clone(),
        set: opt.set.clone(),
        workers: opt.workers,
        use_checksum: true,
        excludes: opt.excludes.clone(),
        prune: false,
        dry_run: false,
        include_system: false,
        cipher: opt.cipher,
        passphrase: opt.passphrase.clone(),
        vss_map: Vec::new(),
        verify: false, // evacuate runs its own verify pass below
    };
    let bstats = backup(&bopt, &progress, &log)?;
    if bstats.errors > 0 {
        return Err(anyhow!("Copy phase had {} error(s) — aborting, NOTHING deleted.", bstats.errors));
    }

    // Verify phase — read everything back and compare hashes.
    log("Verifying copies on destination (BLAKE3) ...");
    let vstats = verify_set(&set_root, opt.passphrase.as_deref(), &progress, &log)?;
    if vstats.errors > 0 {
        return Err(anyhow!("Verification failed for {} file(s) — aborting, NOTHING deleted.", vstats.errors));
    }

    let mut report = EvacuateReport {
        scanned,
        bytes: total_bytes,
        copied: bstats.copied,
        verified: vstats.copied,
        ..Default::default()
    };

    // Delete phase — only reached when copy AND verify are clean.
    if opt.delete_source {
        if opt.secure_wipe {
            log(&format!("Securely wiping source files ({} pass(es)); detecting storage per partition ...",
                opt.wipe_passes.max(1)));
        } else {
            log("Verification OK — deleting source files ...");
        }
        let man = manifest::load(&set_root.join(MANIFEST_NAME))
            .ok_or_else(|| anyhow!("manifest vanished after backup"))?;
        let confirmed: HashSet<&String> = man.files.keys().collect();
        let mut deleted = 0u64;
        let mut wiped = 0u64;
        let mut derr = 0u64;
        let mut parents: HashSet<PathBuf> = HashSet::new();
        // Per-partition detection: group files by the volume they live on, probe
        // the storage type once per partition, and remember an SSD representative.
        let mut vk_cache: HashMap<String, (String, crate::reset::StorageKind)> = HashMap::new();
        let mut part_counts: HashMap<String, (crate::reset::StorageKind, u64)> = HashMap::new();
        let mut ssd_reps: HashMap<String, String> = HashMap::new();
        for e in &entries {
            if !confirmed.contains(&e.rel) {
                continue;
            }
            let mut overwritten = false;
            if opt.secure_wipe && e.meta.kind == Kind::File {
                let abspath = e.abspath.to_string_lossy().into_owned();
                let vk = volume_key(&e.abspath);
                let (label, kind) = vk_cache.entry(vk.clone()).or_insert_with(|| {
                    let (dev, k) = crate::reset::storage_info(&abspath);
                    (if dev.is_empty() { vk.clone() } else { dev }, k)
                }).clone();
                part_counts.entry(label.clone()).or_insert((kind, 0)).1 += 1;
                if kind == crate::reset::StorageKind::Ssd {
                    ssd_reps.entry(label).or_insert(abspath);
                }
                if let Err(err) = secure_overwrite(&e.abspath, opt.wipe_passes) {
                    derr += 1;
                    log(&format!("  WIPE-FAIL {}: {}", e.abspath.display(), err));
                    continue;
                }
                overwritten = true;
            }
            match std::fs::remove_file(&e.abspath) {
                Ok(()) => {
                    deleted += 1;
                    if overwritten { wiped += 1; }
                    if let Some(p) = e.abspath.parent() {
                        parents.insert(p.to_path_buf());
                    }
                }
                Err(err) => {
                    derr += 1;
                    log(&format!("  DELETE-FAIL {}: {}", e.abspath.display(), err));
                }
            }
        }
        // Best-effort prune of now-empty directories (deepest first).
        let mut dirs: Vec<PathBuf> = parents.into_iter().collect();
        dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
        for d in dirs {
            let _ = std::fs::remove_dir(&d);
        }
        report.deleted = deleted;
        report.wiped = wiped;
        report.delete_errors = derr;
        log(&format!("Deleted {deleted} source file(s) ({wiped} securely wiped), {derr} error(s)."));

        // Act per partition: overwrite only reliably destroys data on HDDs.
        if opt.secure_wipe && !part_counts.is_empty() {
            let kinds: HashSet<&str> = part_counts.values().map(|(k, _)| k.as_str()).collect();
            report.storage = if kinds.len() == 1 {
                kinds.iter().next().unwrap().to_string()
            } else {
                "mixed".to_string()
            };
            let mut lines: Vec<(String, String)> = part_counts.iter()
                .map(|(dev, (k, n))| (dev.clone(), format!("  {dev}  [{}]  {n} file(s)", k.as_str())))
                .collect();
            lines.sort();
            log("Storage per partition:");
            for (_, l) in &lines { log(l); }

            let any_non_hdd = part_counts.values().any(|(k, _)| *k != crate::reset::StorageKind::Hdd);
            if any_non_hdd {
                let worst = if kinds.contains("ssd") {
                    crate::reset::StorageKind::Ssd
                } else {
                    crate::reset::StorageKind::Unknown
                };
                log(&crate::reset::secure_erase_advice(worst));
                // Windows: wipe free space on every distinct SSD partition.
                for rep in ssd_reps.values() {
                    if let Some(cmd) = crate::reset::windows_freespace_wipe_command(rep) {
                        log(&format!("SSD on Windows — wiping free space: {}", cmd.join(" ")));
                        match crate::reset::run_command(&cmd) {
                            Ok(c) => log(&format!("free-space wipe exited with code {c}")),
                            Err(e) => log(&format!("free-space wipe failed: {e}")),
                        }
                    }
                }
            } else {
                log(&crate::reset::secure_erase_advice(crate::reset::StorageKind::Hdd));
            }
        }
    }

    Ok(report)
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

    #[test]
    fn to_rel_makes_paths_relative() {
        assert_eq!(to_rel("/Users/x/file"), "Users/x/file");
        assert_eq!(to_rel(r"\\?\C:\Users\x"), r"C\Users\x");
        assert_eq!(to_rel(r"D:\data\f"), r"D\data\f");
        assert_eq!(to_rel(r"C:/Users/x"), "C/Users/x");
    }

    #[test]
    fn auto_sources_nonempty() {
        // dev box has a HOME with at least one standard folder, or falls back to it
        assert!(!auto_sources().is_empty());
    }

    #[test]
    fn volume_root_detection() {
        for p in ["/", "C:\\", "C:", r"\\?\C:\"] {
            assert!(is_volume_root(p), "{p}");
        }
        for p in ["/home/user", "C:\\Users", "/Volumes/Stick"] {
            assert!(!is_volume_root(p), "{p}");
        }
    }

    #[test]
    fn suggested_dest_has_dated_folder() {
        let d = suggested_dest();
        assert!(d.contains("backuptool-"), "got: {d}");
        assert!(!d.is_empty());
    }

    #[test]
    fn scope_config_needs_paths() {
        assert!(scope_sources("config", &[]).is_err());
        assert_eq!(scope_sources("config", &["/x".into()]).unwrap(), vec!["/x".to_string()]);
        assert!(scope_sources("bogus", &[]).is_err());
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let base = std::env::temp_dir()
            .join(format!("bt-test-{}-{}-{:p}", tag, std::process::id(), &tag as *const _));
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn evacuate_moves_and_verifies() {
        let root = tmpdir("evac");
        let src = root.join("src");
        let dest = root.join("dest");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), b"hello").unwrap();
        std::fs::write(src.join("sub/b.txt"), b"world data").unwrap();

        let opt = EvacuateOptions {
            sources: vec![src.to_string_lossy().into_owned()],
            dest: dest.to_string_lossy().into_owned(),
            set: "evacset".into(),
            workers: 2,
            excludes: vec![],
            cipher: Cipher::None,
            passphrase: None,
            delete_source: true,
            secure_wipe: false,
            wipe_passes: 1,
            allow_same_device: true, // temp src+dest share a device
            dry_run: false,
        };
        let report = evacuate(&opt, |_, _| {}, |_| {}).unwrap();

        assert_eq!(report.scanned, 2);
        assert_eq!(report.copied, 2);
        assert_eq!(report.verified, 2);
        assert_eq!(report.verify_errors, 0);
        assert_eq!(report.deleted, 2);
        assert_eq!(report.wiped, 0);
        // sources are gone, copies exist
        assert!(!src.join("a.txt").exists());
        assert!(!src.join("sub/b.txt").exists());

        let set_root = dest.join("evacset");
        let vstats = verify_set(&set_root, None, |_, _| {}, |_| {}).unwrap();
        assert_eq!(vstats.errors, 0);
        assert_eq!(vstats.copied, 2);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn evacuate_dry_run_keeps_sources() {
        let root = tmpdir("evacdry");
        let src = root.join("src");
        let dest = root.join("dest");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("keep.txt"), b"stays").unwrap();

        let opt = EvacuateOptions {
            sources: vec![src.to_string_lossy().into_owned()],
            dest: dest.to_string_lossy().into_owned(),
            set: "s".into(),
            workers: 1,
            excludes: vec![],
            cipher: Cipher::None,
            passphrase: None,
            delete_source: true,
            secure_wipe: false,
            wipe_passes: 1,
            allow_same_device: true,
            dry_run: true,
        };
        let report = evacuate(&opt, |_, _| {}, |_| {}).unwrap();
        assert_eq!(report.scanned, 1);
        assert_eq!(report.deleted, 0);
        assert!(src.join("keep.txt").exists()); // dry-run never deletes

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn evacuate_secure_wipe_removes_sources() {
        let root = tmpdir("evacwipe");
        let src = root.join("src");
        let dest = root.join("dest");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("secret.txt"), b"top secret payload").unwrap();

        let opt = EvacuateOptions {
            sources: vec![src.to_string_lossy().into_owned()],
            dest: dest.to_string_lossy().into_owned(),
            set: "w".into(),
            workers: 1,
            excludes: vec![],
            cipher: Cipher::None,
            passphrase: None,
            delete_source: true,
            secure_wipe: true,
            wipe_passes: 2,
            allow_same_device: true,
            dry_run: false,
        };
        let report = evacuate(&opt, |_, _| {}, |_| {}).unwrap();
        assert_eq!(report.deleted, 1);
        assert_eq!(report.wiped, 1);
        assert!(!src.join("secret.txt").exists());
        // the evacuated copy is intact
        let v = verify_set(&dest.join("w"), None, |_, _| {}, |_| {}).unwrap();
        assert_eq!(v.errors, 0);

        std::fs::remove_dir_all(&root).ok();
    }
}
