// SPDX-License-Identifier: GPL-3.0-or-later
//! Factory-reset guidance and Phase 3 actions.
//!
//! - [`instructions`]: platform-specific manual factory-reset steps.
//! - [`detect_clone_tools`] / [`build_clone_command`]: detect installed
//!   disk-clone utilities and build the command line for the chosen one.
//! - [`factory_reset_command`]: the platform's automatable factory-reset
//!   command, when one exists (Windows); `None` means "do it manually".
//! - [`run_command`]: spawn an external command (used for clone / reset).
//!
//! Anything destructive is invoked by the CLI only behind a typed confirmation,
//! and can be previewed with `--dry-run` (which prints the exact command).

use anyhow::{anyhow, Result};
use std::path::PathBuf;

/// A disk-clone utility candidate and whether it is installed on this system.
#[derive(Debug)]
pub struct CloneTool {
    pub name: &'static str,
    pub description: &'static str,
    pub path: Option<String>,
}

impl CloneTool {
    pub fn available(&self) -> bool {
        self.path.is_some()
    }
}

/// Locate an executable on PATH (like `which`).
pub fn which(prog: &str) -> Option<String> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        let cand: PathBuf = dir.join(prog);
        if cand.is_file() {
            return Some(cand.to_string_lossy().into_owned());
        }
    }
    None
}

fn clone_candidates() -> &'static [(&'static str, &'static str)] {
    if cfg!(target_os = "macos") {
        &[
            ("asr", "Apple Software Restore — block-copy/restore a volume"),
            ("dd", "raw byte copy of a device or file"),
            ("ddrescue", "GNU ddrescue — robust copy, retries bad sectors"),
        ]
    } else if cfg!(target_os = "linux") {
        &[
            ("dd", "raw byte copy of a device or file"),
            ("ddrescue", "GNU ddrescue — robust copy, retries bad sectors"),
            ("partclone.dd", "partclone — filesystem-aware partition clone"),
            ("ntfsclone", "clone NTFS partitions"),
            ("e2image", "clone ext2/3/4 partitions"),
        ]
    } else if cfg!(windows) {
        &[
            ("wbadmin", "Windows Server Backup — system image"),
            ("dd", "raw byte copy (if a dd port is installed)"),
        ]
    } else {
        &[]
    }
}

/// Detect which clone tools are installed on this platform.
pub fn detect_clone_tools() -> Vec<CloneTool> {
    clone_candidates()
        .iter()
        .map(|(name, desc)| CloneTool { name, description: desc, path: which(name) })
        .collect()
}

/// Build the argv for cloning `source` to `target` with the named tool.
/// `source`/`target` are devices or image files. Errors on an unknown tool.
pub fn build_clone_command(tool: &str, source: &str, target: &str) -> Result<Vec<String>> {
    let v: Vec<String> = match tool {
        "dd" => vec![
            "dd".into(), format!("if={source}"), format!("of={target}"),
            "bs=1048576".into(), "conv=noerror,sync".into(),
        ],
        "ddrescue" => vec![
            "ddrescue".into(), "-f".into(), source.into(), target.into(),
            format!("{target}.mapfile"),
        ],
        "asr" => vec![
            "asr".into(), "restore".into(), "--source".into(), source.into(),
            "--target".into(), target.into(), "--erase".into(), "--noprompt".into(),
        ],
        "partclone.dd" => vec![
            "partclone.dd".into(), "-s".into(), source.into(), "-o".into(), target.into(),
        ],
        "ntfsclone" => vec![
            "ntfsclone".into(), "--save-image".into(), "-o".into(), target.into(), source.into(),
        ],
        "e2image" => vec!["e2image".into(), "-r".into(), source.into(), target.into()],
        "wbadmin" => vec![
            "wbadmin".into(), "start".into(), "backup".into(),
            format!("-backupTarget:{target}"), format!("-include:{source}"), "-quiet".into(),
        ],
        other => return Err(anyhow!("unknown clone tool '{other}' (see --list-clone-tools)")),
    };
    Ok(v)
}

/// The platform's automatable factory-reset command, or `None` when a reset can
/// only be performed manually (see [`instructions`]).
pub fn factory_reset_command() -> Option<Vec<String>> {
    if cfg!(windows) {
        Some(vec!["systemreset.exe".into(), "--factoryreset".into()])
    } else {
        None
    }
}

/// Whether a path lives on flash (SSD) or a rotational disk (HDD).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageKind {
    Ssd,
    Hdd,
    Unknown,
}

impl StorageKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            StorageKind::Ssd => "ssd",
            StorageKind::Hdd => "hdd",
            StorageKind::Unknown => "unknown",
        }
    }
}

/// Best-effort detection of the storage type backing `path`.
pub fn storage_type(path: &str) -> StorageKind {
    storage_info(path).1
}

/// Detect the **partition/volume** backing `path` and its storage type, returned
/// as `(device, kind)`. `device` is the partition identifier (e.g. `/dev/disk2s4`,
/// `/dev/sda1`, or a Windows drive letter) so callers can group files per
/// partition on machines with several (possibly mixed SSD/HDD) disks.
pub fn storage_info(path: &str) -> (String, StorageKind) {
    detect_storage_info(path).unwrap_or_else(|| (String::new(), StorageKind::Unknown))
}

#[cfg(target_os = "macos")]
fn detect_storage_info(path: &str) -> Option<(String, StorageKind)> {
    let dev = df_device(path)?;
    let out = std::process::Command::new("diskutil").arg("info").arg(&dev).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
    for line in text.lines() {
        if line.contains("solid state") {
            let kind = if line.contains("yes") { StorageKind::Ssd } else { StorageKind::Hdd };
            return Some((dev, kind));
        }
    }
    Some((dev, StorageKind::Unknown))
}

#[cfg(target_os = "macos")]
fn df_device(path: &str) -> Option<String> {
    let out = std::process::Command::new("df").arg(path).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().nth(1)?.split_whitespace().next().map(|s| s.to_string())
}

#[cfg(target_os = "linux")]
fn detect_storage_info(path: &str) -> Option<(String, StorageKind)> {
    let out = std::process::Command::new("df").arg("--output=source").arg(path).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let dev = text.lines().nth(1)?.trim().to_string();   // the partition, e.g. /dev/sda1
    let name = dev.strip_prefix("/dev/").unwrap_or(&dev);
    let base = base_block_device(name);
    let kind = match std::fs::read_to_string(format!("/sys/block/{base}/queue/rotational")) {
        Ok(r) if r.trim() == "1" => StorageKind::Hdd,
        Ok(_) => StorageKind::Ssd,
        Err(_) => StorageKind::Unknown,
    };
    Some((dev, kind))
}

#[cfg(target_os = "linux")]
fn base_block_device(name: &str) -> String {
    // nvme0n1p2 -> nvme0n1 ; mmcblk0p1 -> mmcblk0 ; sda1 -> sda
    if (name.starts_with("nvme") || name.starts_with("mmcblk")) && name.contains('p') {
        if let Some(idx) = name.rfind('p') {
            return name[..idx].to_string();
        }
    }
    name.trim_end_matches(|c: char| c.is_ascii_digit()).to_string()
}

#[cfg(target_os = "windows")]
fn detect_storage_info(path: &str) -> Option<(String, StorageKind)> {
    let drive = path.chars().next()?;
    let ps = format!(
        "$ErrorActionPreference='SilentlyContinue';(Get-Partition -DriveLetter '{drive}' | Get-Disk | Get-PhysicalDisk).MediaType"
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &ps]).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).to_lowercase();
    let kind = if s.contains("ssd") { StorageKind::Ssd }
        else if s.contains("hdd") { StorageKind::Hdd }
        else { StorageKind::Unknown };
    Some((format!("{drive}:"), kind))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn detect_storage_info(_path: &str) -> Option<(String, StorageKind)> {
    None
}

/// Advice on how to securely erase, given the detected storage type. Overwrite
/// is only trustworthy on HDDs; otherwise we point at the platform's own tool.
pub fn secure_erase_advice(kind: StorageKind) -> String {
    let head = match kind {
        StorageKind::Hdd => "HDD detected — multi-pass overwrite reliably destroys the data.",
        StorageKind::Ssd => "SSD/flash detected — per-file overwrite is NOT reliable (wear-leveling keeps old copies).",
        StorageKind::Unknown => "Storage type unknown — treat overwrite as best-effort only.",
    };
    if kind == StorageKind::Hdd {
        return head.to_string();
    }
    let tool = if cfg!(windows) {
        "Windows: run 'cipher /w:<volume>' to wipe free space, turn on BitLocker (crypto-erase),\n  or use the drive vendor's ATA Secure Erase utility."
    } else if cfg!(target_os = "linux") {
        "Linux: use 'blkdiscard' (TRIM), 'hdparm --security-erase', or 'nvme format -s2' on the whole device;\n  or full-disk encryption (LUKS) + key destruction."
    } else if cfg!(target_os = "macos") {
        "macOS: use 'diskutil secureErase', or rely on FileVault crypto-erase ('Erase All Content and Settings')."
    } else {
        "Use your platform's hardware secure-erase tool."
    };
    format!("{head}\n  {tool}")
}

/// On Windows, the command to overwrite a volume's free space (built-in
/// `cipher /w`). `None` on other platforms (they have their own tools).
pub fn windows_freespace_wipe_command(path: &str) -> Option<Vec<String>> {
    if cfg!(windows) {
        Some(vec!["cipher".into(), format!("/w:{path}")])
    } else {
        None
    }
}

/// Spawn an external command (inheriting stdio) and return its exit code.
pub fn run_command(argv: &[String]) -> Result<i32> {
    let (prog, args) = argv.split_first().ok_or_else(|| anyhow!("empty command"))?;
    let status = std::process::Command::new(prog).args(args).status()
        .map_err(|e| anyhow!("failed to run '{prog}': {e}"))?;
    Ok(status.code().unwrap_or(-1))
}

/// Human-readable, platform-specific factory-reset instructions.
pub fn instructions() -> String {
    if cfg!(target_os = "macos") {
        "macOS: a factory reset cannot be scripted by third-party tools.\n\
         Do it manually:\n\
         - Apple silicon / T2 Macs: System Settings > General > Transfer or Reset\n\
           > Erase All Content and Settings.\n\
         - Older Macs: reboot into Recovery (hold Cmd-R), use Disk Utility to erase\n\
           the system volume, then reinstall macOS."
            .to_string()
    } else if cfg!(target_os = "linux") {
        "Linux: there is no universal factory reset.\n\
         Typical options:\n\
         - Appliance/immutable systems: trigger the vendor reset (e.g. reset an\n\
           OverlayFS upper layer, or re-flash the factory image).\n\
         - Desktop installs: reinstall from your installation medium, or remove\n\
           user data (/home, /root) and reset configuration to defaults."
            .to_string()
    } else if cfg!(windows) {
        "Windows: use the built-in reset.\n\
         - GUI: Settings > System > Recovery > Reset this PC > Remove everything.\n\
         - CLI (elevated): systemreset.exe --factoryreset"
            .to_string()
    } else {
        "Factory reset is platform-specific; consult your device documentation.".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dd_command_is_well_formed() {
        let cmd = build_clone_command("dd", "/dev/disk2", "/mnt/img.raw").unwrap();
        assert_eq!(cmd[0], "dd");
        assert!(cmd.contains(&"if=/dev/disk2".to_string()));
        assert!(cmd.contains(&"of=/mnt/img.raw".to_string()));
    }

    #[test]
    fn unknown_clone_tool_errors() {
        assert!(build_clone_command("bogus", "a", "b").is_err());
    }

    #[test]
    fn detection_lists_candidates() {
        // dd exists on every unix dev box; at least one candidate should resolve.
        let tools = detect_clone_tools();
        assert!(!tools.is_empty());
        if cfg!(unix) {
            assert!(tools.iter().any(|t| t.name == "dd" && t.available()));
        }
    }

    #[test]
    fn macos_reset_is_manual() {
        if cfg!(target_os = "macos") {
            assert!(factory_reset_command().is_none());
        }
    }

    #[test]
    fn storage_type_detects_root() {
        // Always returns a valid kind without panicking. Only require a *concrete*
        // kind on macOS: Linux CI containers back "/" with overlayfs, which has no
        // backing disk to classify, so "unknown" is legitimate there.
        let k = storage_type("/");
        if cfg!(target_os = "macos") {
            assert_ne!(k, StorageKind::Unknown);
        }
    }

    #[test]
    fn storage_info_names_partition() {
        // As above: only require a named partition device on macOS. On Linux CI
        // (overlayfs root) there may be no backing device to report.
        let (dev, kind) = storage_info("/");
        if cfg!(target_os = "macos") {
            assert!(!dev.is_empty(), "expected a partition device for /");
            assert_ne!(kind, StorageKind::Unknown);
        }
        let _ = (dev, kind);
    }

    #[test]
    fn advice_mentions_a_tool_for_ssd() {
        let a = secure_erase_advice(StorageKind::Ssd);
        assert!(a.to_lowercase().contains("cipher")
            || a.to_lowercase().contains("blkdiscard")
            || a.to_lowercase().contains("diskutil"));
        // HDD advice says overwrite is enough.
        assert!(secure_erase_advice(StorageKind::Hdd).to_lowercase().contains("overwrite"));
    }
}
