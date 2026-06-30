// SPDX-License-Identifier: GPL-3.0-or-later
//! Detect (and optionally close) running applications that commonly hold user
//! files open, so a file-level backup doesn't fail with "in use" errors.
//! Cross-platform: tasklist on Windows, ps elsewhere.

/// (process token, friendly name). The token is matched against the running
/// process list (as "<token>.exe" on Windows, as a substring elsewhere).
const BLOCKERS: &[(&str, &str)] = &[
    ("firefox", "Firefox"),
    ("chrome", "Google Chrome"),
    ("msedge", "Microsoft Edge"),
    ("chromium", "Chromium"),
    ("brave", "Brave"),
    ("opera", "Opera"),
    ("vivaldi", "Vivaldi"),
    ("safari", "Safari"),
    ("thunderbird", "Thunderbird"),
    ("photoshop", "Photoshop"),
    ("illustrator", "Illustrator"),
    ("indesign", "InDesign"),
    ("lightroom", "Lightroom"),
    ("figma", "Figma"),
    ("acrobat", "Acrobat"),
    ("acrord32", "Acrobat Reader"),
    ("winword", "Word"),
    ("excel", "Excel"),
    ("powerpnt", "PowerPoint"),
    ("outlook", "Outlook"),
    ("onenote", "OneNote"),
    ("onedrive", "OneDrive"),
    ("dropbox", "Dropbox"),
    ("spotify", "Spotify"),
];

fn process_list_lower() -> String {
    #[cfg(windows)]
    let cmd = std::process::Command::new("tasklist").output();
    #[cfg(not(windows))]
    let cmd = std::process::Command::new("ps").args(["-A", "-o", "comm="]).output();
    cmd.ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase())
        .unwrap_or_default()
}

fn token_running(list: &str, token: &str) -> bool {
    #[cfg(windows)]
    {
        list.contains(&format!("{token}.exe"))
    }
    #[cfg(not(windows))]
    {
        list.contains(token)
    }
}

/// Friendly names of running apps likely to lock files (deduplicated).
pub fn running_blockers() -> Vec<String> {
    let list = process_list_lower();
    let mut found: Vec<String> = Vec::new();
    for (tok, name) in BLOCKERS {
        if token_running(&list, tok) && !found.iter().any(|n| n == name) {
            found.push((*name).to_string());
        }
    }
    found
}

/// Ask each running blocker to close (graceful: lets apps prompt to save).
/// Returns the friendly names it signalled. Re-check with [`running_blockers`].
pub fn close_apps() -> Vec<String> {
    let list = process_list_lower();
    let mut closed: Vec<String> = Vec::new();
    for (tok, name) in BLOCKERS {
        if !token_running(&list, tok) {
            continue;
        }
        #[cfg(windows)]
        {
            // graceful close request (no /F) so unsaved work can be saved
            let _ = std::process::Command::new("taskkill")
                .args(["/IM", &format!("{tok}.exe")])
                .output();
        }
        #[cfg(not(windows))]
        {
            let _ = std::process::Command::new("pkill").args(["-i", tok]).output();
        }
        if !closed.iter().any(|n| n == name) {
            closed.push((*name).to_string());
        }
    }
    closed
}

#[cfg(test)]
mod tests {
    #[test]
    fn running_blockers_does_not_panic() {
        // returns a (possibly empty) list without crashing
        let _ = super::running_blockers();
    }
}
