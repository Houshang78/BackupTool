// SPDX-License-Identifier: GPL-3.0-or-later
//! Windows Volume Shadow Copy (VSS) — snapshot a volume so the file backup can
//! read files that are locked/open by other processes (Defender, the search
//! indexer, running apps). No-op on other platforms.
//!
//! Flow: [`create`] a snapshot of a volume → read files from its device path
//! (`\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopyN`) → [`remove`] it after.
//! Creating a shadow copy needs Administrator rights.

#[derive(Debug, Clone)]
pub struct Shadow {
    pub id: String,
    /// Device path of the snapshot, e.g. \\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy3
    pub device: String,
    /// The original volume, e.g. "C:"
    pub volume: String,
}

/// Result of [`prepare`]: (scan_sources, vss_map as [(device, volume)], created_shadows).
pub type Prepared = (Vec<String>, Vec<(String, String)>, Vec<Shadow>);

/// Drive-letter volume of a path ("C:\\Users" -> "C:"), or None.
pub fn volume_of(path: &str) -> Option<String> {
    let b = path.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        Some(format!("{}:", b[0] as char))
    } else {
        None
    }
}

/// Snapshot each distinct source volume and remap sources to read from the
/// snapshot. Returns (scan_sources, vss_map, created_shadows). Errors (no admin
/// / not Windows) propagate so the caller can fall back to a normal copy.
pub fn prepare(
    sources: &[String],
) -> anyhow::Result<Prepared> {
    use std::collections::HashMap;
    let mut shadows: HashMap<String, Shadow> = HashMap::new();
    let mut created = Vec::new();
    let mut new_sources = Vec::new();
    let mut map: Vec<(String, String)> = Vec::new();
    for s in sources {
        match volume_of(s) {
            Some(vol) => {
                let sh = match shadows.get(&vol) {
                    Some(sh) => sh.clone(),
                    None => {
                        let sh = create(&vol)?;
                        shadows.insert(vol.clone(), sh.clone());
                        created.push(sh.clone());
                        map.push((format!("{}\\", sh.device), vol.clone()));
                        sh
                    }
                };
                let suffix = s[vol.len()..].trim_start_matches(['\\', '/']);
                new_sources.push(format!("{}\\{}", sh.device, suffix));
            }
            None => new_sources.push(s.clone()),
        }
    }
    Ok((new_sources, map, created))
}

#[cfg(windows)]
pub fn create(volume: &str) -> anyhow::Result<Shadow> {
    use anyhow::{anyhow, bail};
    let drive = volume.trim_end_matches(['\\', '/']); // "C:"
    let ps = format!(
        "$ErrorActionPreference='Stop';\
         $r=([wmiclass]'Win32_ShadowCopy').Create('{drive}\\','ClientAccessible');\
         $s=Get-WmiObject Win32_ShadowCopy | Where-Object {{ $_.ID -eq $r.ShadowID }};\
         Write-Output ($s.ID + '|' + $s.DeviceObject)"
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .output()
        .map_err(|e| anyhow!("could not run powershell: {e}"))?;
    if !out.status.success() {
        bail!("VSS snapshot of {drive} failed (needs admin): {}",
            String::from_utf8_lossy(&out.stderr).trim());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.contains('|'))
        .ok_or_else(|| anyhow!("VSS: unexpected output: {}", text.trim()))?;
    let mut it = line.trim().splitn(2, '|');
    let id = it.next().unwrap_or_default().to_string();
    let device = it.next().unwrap_or_default().to_string();
    if id.is_empty() || device.is_empty() {
        bail!("VSS: could not parse snapshot id/device");
    }
    Ok(Shadow { id, device, volume: drive.to_string() })
}

#[cfg(windows)]
pub fn remove(id: &str) {
    let ps = format!(
        "(Get-WmiObject Win32_ShadowCopy | Where-Object {{ $_.ID -eq '{id}' }}).Delete()"
    );
    let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .output();
}

#[cfg(not(windows))]
pub fn create(_volume: &str) -> anyhow::Result<Shadow> {
    anyhow::bail!("VSS snapshots are only available on Windows")
}

#[cfg(not(windows))]
pub fn remove(_id: &str) {}
