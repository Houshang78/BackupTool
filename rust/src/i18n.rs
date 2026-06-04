// SPDX-License-Identifier: GPL-3.0-or-later
//! Minimal i18n for the GUI.
//!
//! EN/DE/FA catalogs are embedded at build time. Additional languages can be
//! added at runtime by dropping a `<code>.json` file into a `lang/` folder next
//! to the executable (or in the current directory) – it is picked up on start.

use serde_json::{Map, Value};
use std::collections::BTreeMap;

fn builtin() -> [(&'static str, &'static str); 3] {
    [
        ("en", include_str!("../lang/en.json")),
        ("de", include_str!("../lang/de.json")),
        ("fa", include_str!("../lang/fa.json")),
    ]
}

pub struct I18n {
    catalogs: BTreeMap<String, Map<String, Value>>,
    en: Map<String, Value>,
}

fn parse(s: &str) -> Option<Map<String, Value>> {
    match serde_json::from_str::<Value>(s) {
        Ok(Value::Object(m)) => Some(m),
        _ => None,
    }
}

impl I18n {
    pub fn load() -> Self {
        let mut catalogs = BTreeMap::new();
        for (code, s) in builtin() {
            if let Some(m) = parse(s) {
                catalogs.insert(code.to_string(), m);
            }
        }
        // Runtime extras: lang/*.json next to the binary or in the current dir.
        let mut dirs: Vec<std::path::PathBuf> = vec![std::path::PathBuf::from("lang")];
        if let Ok(exe) = std::env::current_exe() {
            if let Some(p) = exe.parent() {
                dirs.push(p.join("lang"));
            }
        }
        for dir in dirs {
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    let path = e.path();
                    if path.extension().and_then(|x| x.to_str()) == Some("json") {
                        if let (Some(stem), Ok(text)) =
                            (path.file_stem().and_then(|s| s.to_str()), std::fs::read_to_string(&path))
                        {
                            if let Some(m) = parse(&text) {
                                catalogs.insert(stem.to_string(), m);
                            }
                        }
                    }
                }
            }
        }
        let en = catalogs.get("en").cloned().unwrap_or_default();
        I18n { catalogs, en }
    }

    /// Available language codes, sorted (e.g. ["de", "en", "fa"]).
    pub fn codes(&self) -> Vec<String> {
        self.catalogs.keys().cloned().collect()
    }

    pub fn name_of(&self, code: &str) -> String {
        self.tr(code, "language_name")
    }

    pub fn is_rtl(&self, code: &str) -> bool {
        self.catalogs
            .get(code)
            .and_then(|m| m.get("rtl"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// Translate a key for the given language, falling back to English then key.
    pub fn tr(&self, code: &str, key: &str) -> String {
        self.catalogs
            .get(code)
            .and_then(|m| m.get(key))
            .and_then(|v| v.as_str())
            .or_else(|| self.en.get(key).and_then(|v| v.as_str()))
            .unwrap_or(key)
            .to_string()
    }
}
