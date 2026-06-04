// SPDX-License-Identifier: GPL-3.0-or-later
//! Slint GUI (native). Build:  cargo run --features gui --bin backuptool-gui
//!
//! Choose sources/destination/set/workers, the BLAKE3 compare and the encryption
//! (none / AES-256-GCM / ChaCha20-Poly1305) from a dropdown. The backup runs in a
//! background thread; progress/log come back through the event loop. UI strings are
//! loaded from the embedded EN/DE/FA catalogs (plus any extra lang/*.json).

slint::include_modules!();

use backuptool::crypto::Cipher;
use backuptool::engine::{self, BackupOptions};
use backuptool::i18n::I18n;
use slint::{ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

type Handle = Arc<Mutex<slint::Weak<MainWindow>>>;

/// Run a closure on the UI thread with the upgraded window (from a worker thread).
fn ui_apply<F: FnOnce(MainWindow) + Send + 'static>(handle: &Handle, f: F) {
    let weak = handle.lock().unwrap().clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            f(ui);
        }
    });
}

/// Apply all translated label texts for the given language code.
fn apply_language(ui: &MainWindow, i18n: &I18n, code: &str) {
    ui.set_win_title(format!("backuptool — {}", i18n.name_of(code)).into());
    ui.set_t_language(i18n.tr(code, "language").into());
    ui.set_t_sources(i18n.tr(code, "sources").into());
    ui.set_t_pick(i18n.tr(code, "choose").into());
    ui.set_t_dest(i18n.tr(code, "dest").into());
    ui.set_t_set(i18n.tr(code, "set_name").into());
    ui.set_t_workers(i18n.tr(code, "workers").into());
    ui.set_t_checksum(i18n.tr(code, "opt_checksum").into());
    ui.set_t_delete(i18n.tr(code, "opt_delete").into());
    ui.set_t_encryption(i18n.tr(code, "encryption").into());
    ui.set_t_password(i18n.tr(code, "password").into());
    ui.set_t_start(i18n.tr(code, "start_backup").into());
    ui.set_t_running(i18n.tr(code, "running").into());
    ui.set_status(i18n.tr(code, "ready").into());
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ui = MainWindow::new()?;
    let i18n = Rc::new(I18n::load());
    let codes: Vec<String> = i18n.codes();

    // Populate the language dropdown with display names.
    let names: Vec<SharedString> = codes.iter().map(|c| i18n.name_of(c).into()).collect();
    ui.set_languages(ModelRc::new(VecModel::from(names)));

    // Default language: English if present, otherwise the first one.
    let default_idx = codes.iter().position(|c| c == "en").unwrap_or(0);
    let current = Rc::new(RefCell::new(codes.get(default_idx).cloned().unwrap_or_else(|| "en".into())));
    ui.set_lang_index(default_idx as i32);
    apply_language(&ui, &i18n, &current.borrow());

    ui.set_setname(gethostname::gethostname().to_string_lossy().into_owned().into());
    ui.set_workers(std::thread::available_parallelism().map(|n| n.get() as i32).unwrap_or(4));

    // Language switch
    {
        let weak = ui.as_weak();
        let i18n = i18n.clone();
        let codes = codes.clone();
        let current = current.clone();
        ui.on_language_changed(move |idx| {
            if let (Some(ui), Some(code)) = (weak.upgrade(), codes.get(idx as usize)) {
                *current.borrow_mut() = code.clone();
                apply_language(&ui, &i18n, code);
            }
        });
    }

    // Pick a source folder -> append to the sources list
    {
        let weak = ui.as_weak();
        ui.on_pick_source(move || {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                if let Some(ui) = weak.upgrade() {
                    let mut s = ui.get_sources().to_string();
                    if !s.is_empty() {
                        s.push(';');
                    }
                    s.push_str(&dir.to_string_lossy());
                    ui.set_sources(s.into());
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_pick_dest(move || {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                if let Some(ui) = weak.upgrade() {
                    ui.set_dest(dir.to_string_lossy().into_owned().into());
                }
            }
        });
    }

    // Start backup
    {
        let weak = ui.as_weak();
        let i18n = i18n.clone();
        let current = current.clone();
        ui.on_start_backup(move || {
            let ui = match weak.upgrade() { Some(u) => u, None => return };
            let code = current.borrow().clone();
            let tr = |k: &str| i18n.tr(&code, k);

            let cipher = match Cipher::parse(&ui.get_cipher()) {
                Ok(c) => c,
                Err(e) => { ui.set_status(format!("{}: {e}", tr("error")).into()); return; }
            };
            let pass = ui.get_passphrase().to_string();
            if cipher != Cipher::None && pass.is_empty() {
                ui.set_status(tr("need_password").into());
                return;
            }
            let sources: Vec<String> = ui.get_sources().split(';')
                .map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            if sources.is_empty() { ui.set_status(tr("need_source").into()); return; }
            let dest = ui.get_dest().to_string();
            if dest.is_empty() { ui.set_status(tr("need_dest").into()); return; }

            let opt = BackupOptions {
                sources,
                dest,
                set: ui.get_setname().to_string(),
                workers: ui.get_workers().max(1) as usize,
                use_checksum: ui.get_checksum(),
                excludes: vec![],
                prune: ui.get_delete_missing(),
                dry_run: false,
                cipher,
                passphrase: if cipher != Cipher::None { Some(pass) } else { None },
            };

            ui.set_running(true);
            ui.set_logtext("".into());
            ui.set_progress(0.0);
            ui.set_status(tr("running").into());

            let handle: Handle = Arc::new(Mutex::new(ui.as_weak()));
            let (h_prog, h_log, h_fin) = (handle.clone(), handle.clone(), handle.clone());

            std::thread::spawn(move || {
                let progress = move |d: u64, t: u64| {
                    let frac = if t > 0 { d as f32 / t as f32 } else { 0.0 };
                    ui_apply(&h_prog, move |ui| {
                        ui.set_progress(frac);
                        ui.set_status(format!("{d}/{t}").into());
                    });
                };
                let log = move |m: &str| {
                    let line = m.to_string();
                    ui_apply(&h_log, move |ui| {
                        let mut t = ui.get_logtext().to_string();
                        t.push_str(&line);
                        t.push('\n');
                        ui.set_logtext(t.into());
                    });
                };
                let msg = match engine::backup(&opt, progress, log) {
                    Ok(s) => format!("{} copied, {} skipped, {} errors.",
                                     s.copied, s.skipped, s.errors),
                    Err(e) => format!("{e}"),
                };
                ui_apply(&h_fin, move |ui| {
                    ui.set_running(false);
                    ui.set_status(msg.into());
                });
            });
        });
    }

    ui.run()?;
    Ok(())
}
