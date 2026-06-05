// SPDX-License-Identifier: GPL-3.0-or-later
//! Discovery: candidate sources (user/service data) and destinations
//! (USB/external/network). Standard library only.

use std::collections::HashSet;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Candidate {
    pub label: String,
    pub path: String,
    /// sources: user|service|data|config — destinations: usb|external|network
    pub kind: String,
}

fn dedup_existing(cands: Vec<Candidate>) -> Vec<Candidate> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for c in cands {
        let real = std::fs::canonicalize(&c.path)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| c.path.clone());
        if Path::new(&real).is_dir() && seen.insert(real.clone()) {
            out.push(Candidate { path: real, ..c });
        }
    }
    out
}

// ------------------------------------------------------------------ Linux
#[cfg(target_os = "linux")]
fn sources() -> Vec<Candidate> {
    const NOLOGIN: &[&str] = &["/usr/sbin/nologin", "/sbin/nologin", "/bin/false", "/usr/bin/false", ""];
    const TRIVIAL: &[&str] = &["/", "/nonexistent", "/dev/null", "/bin", "/sbin", "/usr/sbin", "/var/empty"];
    let mut v = Vec::new();
    if let Ok(txt) = std::fs::read_to_string("/etc/passwd") {
        for line in txt.lines() {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() < 7 {
                continue;
            }
            let (name, home, shell) = (f[0], f[5], f[6]);
            let uid: u32 = f[2].parse().unwrap_or(u32::MAX);
            if (1000..65534).contains(&uid) && home.starts_with("/home") {
                v.push(Candidate { label: format!("User: {name}"), path: home.into(), kind: "user".into() });
            } else if uid < 1000 && !NOLOGIN.contains(&shell) && !TRIVIAL.contains(&home)
                && (home.starts_with("/var") || home.starts_with("/srv") || home.starts_with("/opt"))
            {
                v.push(Candidate { label: format!("Service: {name}"), path: home.into(), kind: "service".into() });
            }
        }
    }
    v.push(Candidate { label: "root".into(), path: "/root".into(), kind: "user".into() });
    for p in ["/srv", "/var/www", "/opt"] {
        v.push(Candidate { label: p.into(), path: p.into(), kind: "data".into() });
    }
    v.push(Candidate { label: "System config (/etc)".into(), path: "/etc".into(), kind: "config".into() });
    dedup_existing(v)
}

#[cfg(target_os = "linux")]
fn removable(dev: &str) -> bool {
    let base: String = Path::new(dev)
        .file_name()
        .map(|s| s.to_string_lossy().trim_end_matches(|c: char| c.is_ascii_digit()).to_string())
        .unwrap_or_default();
    std::fs::read_to_string(format!("/sys/block/{base}/removable"))
        .map(|s| s.trim() == "1")
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn destinations() -> Vec<Candidate> {
    const NET: &[&str] = &["nfs", "nfs4", "cifs", "smbfs", "smb3", "fuse.sshfs", "ncpfs", "afs", "9p"];
    let mut v = Vec::new();
    if let Ok(txt) = std::fs::read_to_string("/proc/mounts") {
        for line in txt.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 3 {
                continue;
            }
            let dev = f[0];
            let mnt = f[1].replace("\\040", " ");
            let fstype = f[2];
            if NET.contains(&fstype) {
                v.push(Candidate { label: format!("Network: {mnt}"), path: mnt, kind: "network".into() });
            } else if (mnt.starts_with("/media/") || mnt.starts_with("/run/media/") || mnt.starts_with("/mnt/"))
                && dev.starts_with("/dev/")
            {
                let usb = removable(dev);
                v.push(Candidate {
                    label: format!("{}: {mnt}", if usb { "USB" } else { "Disk" }),
                    path: mnt,
                    kind: if usb { "usb" } else { "external" }.into(),
                });
            }
        }
    }
    dedup_existing(v)
}

// ------------------------------------------------------------------ macOS
#[cfg(target_os = "macos")]
fn sources() -> Vec<Candidate> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/Users") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if ["Shared", "Guest", ".localized"].contains(&n.as_str()) {
                continue;
            }
            v.push(Candidate { label: format!("User: {n}"), path: format!("/Users/{n}"), kind: "user".into() });
        }
    }
    for p in ["/usr/local/var", "/opt/homebrew/var", "/Library", "/srv"] {
        v.push(Candidate { label: p.into(), path: p.into(), kind: "data".into() });
    }
    v.push(Candidate { label: "System config (/etc)".into(), path: "/etc".into(), kind: "config".into() });
    dedup_existing(v)
}

#[cfg(target_os = "macos")]
fn destinations() -> Vec<Candidate> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/Volumes") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            v.push(Candidate { label: format!("Volume: {n}"), path: format!("/Volumes/{n}"), kind: "external".into() });
        }
    }
    dedup_existing(v)
}

// ------------------------------------------------------------------ Windows
#[cfg(target_os = "windows")]
fn sources() -> Vec<Candidate> {
    let mut v = Vec::new();
    let sysdrive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let base = format!("{sysdrive}\\Users");
    if let Ok(rd) = std::fs::read_dir(&base) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if ["public", "default", "default user", "all users"].contains(&n.to_lowercase().as_str()) {
                continue;
            }
            v.push(Candidate { label: format!("User: {n}"), path: format!("{base}\\{n}"), kind: "user".into() });
        }
    }
    if let Ok(pd) = std::env::var("ProgramData") {
        v.push(Candidate { label: "ProgramData".into(), path: pd, kind: "data".into() });
    }
    dedup_existing(v)
}

// Drive-type detection needs WinAPI; left best-effort empty to avoid extra deps.
#[cfg(target_os = "windows")]
fn destinations() -> Vec<Candidate> {
    Vec::new()
}

// ------------------------------------------------------------------ other
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn sources() -> Vec<Candidate> {
    Vec::new()
}
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn destinations() -> Vec<Candidate> {
    Vec::new()
}

// ------------------------------------------------------------------ public API
pub fn user_data_sources() -> Vec<Candidate> {
    sources()
}

pub fn detect_destinations() -> Vec<Candidate> {
    destinations()
}

/// Best default destination: USB first, then external disk, then network.
pub fn default_destination() -> Option<String> {
    let d = detect_destinations();
    for kind in ["usb", "external", "network"] {
        if let Some(c) = d.iter().find(|c| c.kind == kind) {
            return Some(c.path.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_are_real_dirs() {
        for c in user_data_sources() {
            assert!(Path::new(&c.path).is_dir(), "not a dir: {}", c.path);
            assert!(!c.label.is_empty());
        }
    }

    #[test]
    fn destinations_have_valid_kind() {
        for c in detect_destinations() {
            assert!(["usb", "external", "network"].contains(&c.kind.as_str()));
        }
    }

    #[test]
    fn default_destination_is_optional() {
        let _ = default_destination();
    }
}
