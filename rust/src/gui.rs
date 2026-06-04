// SPDX-License-Identifier: GPL-3.0-or-later
//! Slint GUI (native). Build:  cargo run --features gui --bin backuptool-gui
//!
//! Two tabs (Backup / Restore). Choose sources/destination/set/workers, the BLAKE3
//! compare and the encryption (none / AES-256-GCM / ChaCha20-Poly1305) from a
//! dropdown. Jobs run in a background thread; progress/log come back through the
//! event loop. UI strings are loaded from the embedded EN/DE/FA catalogs (plus any
//! extra lang/*.json).

slint::include_modules!();

use backuptool::engine::{self, BackupOptions, RestoreOptions};
use backuptool::i18n::I18n;
use slint::{Model, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use backuptool::crypto::Cipher;

type Handle = Arc<Mutex<slint::Weak<MainWindow>>>;
type Prog = Box<dyn Fn(u64, u64) + Send + Sync>;
type Log = Box<dyn Fn(&str) + Send + Sync>;

/// Run a closure on the UI thread with the upgraded window (from a worker thread).
fn ui_apply<F: FnOnce(MainWindow) + Send + 'static>(handle: &Handle, f: F) {
    let weak = handle.lock().unwrap().clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            f(ui);
        }
    });
}

/// Spawn a background job that reports progress/log to the UI and a final message.
fn spawn_job<J>(ui: &MainWindow, job: J)
where
    J: FnOnce(Prog, Log) -> String + Send + 'static,
{
    ui.set_running(true);
    ui.set_logtext("".into());
    ui.set_progress(0.0);

    let handle: Handle = Arc::new(Mutex::new(ui.as_weak()));
    let (h_prog, h_log, h_fin) = (handle.clone(), handle.clone(), handle.clone());

    std::thread::spawn(move || {
        let progress: Prog = Box::new(move |d, t| {
            let frac = if t > 0 { d as f32 / t as f32 } else { 0.0 };
            ui_apply(&h_prog, move |ui| {
                ui.set_progress(frac);
                ui.set_status(format!("{d}/{t}").into());
            });
        });
        let log: Log = Box::new(move |m: &str| {
            let line = m.to_string();
            ui_apply(&h_log, move |ui| {
                let mut t = ui.get_logtext().to_string();
                t.push_str(&line);
                t.push('\n');
                ui.set_logtext(t.into());
            });
        });
        let msg = job(progress, log);
        ui_apply(&h_fin, move |ui| {
            ui.set_running(false);
            ui.set_status(msg.into());
        });
    });
}

/// Apply all translated label texts for the given language code.
fn apply_language(ui: &MainWindow, i18n: &I18n, code: &str) {
    let t = |k: &str| i18n.tr(code, k);
    ui.set_win_title(format!("backuptool — {}", i18n.name_of(code)).into());
    ui.set_t_language(t("language").into());
    // backup tab
    ui.set_t_tab_backup(t("tab_backup").into());
    ui.set_t_sources(t("sources").into());
    ui.set_t_pick(t("choose").into());
    ui.set_t_dest(t("dest").into());
    ui.set_t_set(t("set_name").into());
    ui.set_t_workers(t("workers").into());
    ui.set_t_checksum(t("opt_checksum").into());
    ui.set_t_delete(t("opt_delete").into());
    ui.set_t_encryption(t("encryption").into());
    ui.set_t_password(t("password").into());
    ui.set_t_start(t("start_backup").into());
    ui.set_t_running(t("running").into());
    ui.set_t_dry(t("opt_dryrun").into());
    ui.set_t_system(t("opt_system").into());
    // restore tab
    ui.set_t_tab_restore(t("tab_restore").into());
    ui.set_t_folder(t("backup_folder").into());
    ui.set_t_rset(t("backup_set").into());
    ui.set_t_loadsets(t("load_sets").into());
    ui.set_t_target(t("target_root").into());
    ui.set_t_reapply(t("reapply_meta").into());
    ui.set_t_start_restore(t("start_restore").into());
    ui.set_status(t("ready").into());
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ui = MainWindow::new()?;
    let i18n = Rc::new(I18n::load());
    let codes: Vec<String> = i18n.codes();

    let names: Vec<SharedString> = codes.iter().map(|c| i18n.name_of(c).into()).collect();
    ui.set_languages(ModelRc::new(VecModel::from(names)));

    let default_idx = codes.iter().position(|c| c == "en").unwrap_or(0);
    let current = Rc::new(RefCell::new(codes.get(default_idx).cloned().unwrap_or_else(|| "en".into())));
    ui.set_lang_index(default_idx as i32);
    apply_language(&ui, &i18n, &current.borrow());

    ui.set_setname(gethostname::gethostname().to_string_lossy().into_owned().into());
    let cores = std::thread::available_parallelism().map(|n| n.get() as i32).unwrap_or(4);
    ui.set_workers(cores);
    ui.set_r_workers(cores);

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

    // --- backup tab folder pickers ---
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

    // --- backup ---
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
                include_system: ui.get_system(),
                dry_run: false,
                cipher,
                passphrase: if cipher != Cipher::None { Some(pass) } else { None },
            };
            ui.set_status(tr("running").into());
            spawn_job(&ui, move |progress, log| match engine::backup(&opt, progress, log) {
                Ok(s) => format!("{} copied, {} skipped, {} errors.", s.copied, s.skipped, s.errors),
                Err(e) => format!("{e}"),
            });
        });
    }

    // --- restore tab pickers ---
    {
        let weak = ui.as_weak();
        ui.on_pick_folder(move || {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                if let Some(ui) = weak.upgrade() {
                    ui.set_r_folder(dir.to_string_lossy().into_owned().into());
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_pick_target(move || {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                if let Some(ui) = weak.upgrade() {
                    ui.set_r_target(dir.to_string_lossy().into_owned().into());
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_load_sets(move || {
            if let Some(ui) = weak.upgrade() {
                let sets = engine::list_sets(&ui.get_r_folder());
                let names: Vec<SharedString> = sets.iter().map(|(s, _, _, _)| s.clone().into()).collect();
                ui.set_r_sets(ModelRc::new(VecModel::from(names)));
                ui.set_r_set_index(0);
            }
        });
    }

    // --- restore ---
    {
        let weak = ui.as_weak();
        let i18n = i18n.clone();
        let current = current.clone();
        ui.on_start_restore(move || {
            let ui = match weak.upgrade() { Some(u) => u, None => return };
            let code = current.borrow().clone();
            let tr = |k: &str| i18n.tr(&code, k);

            let folder = ui.get_r_folder().to_string();
            if folder.is_empty() { ui.set_status(tr("backup_folder").into()); return; }
            let idx = ui.get_r_set_index().max(0) as usize;
            let setname = match ui.get_r_sets().row_data(idx) {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => { ui.set_status(tr("backup_set").into()); return; }
            };

            // Detect encryption from the manifest -> require a password if needed.
            let set_root = std::path::Path::new(&folder).join(&setname);
            let man = backuptool::manifest::load(&set_root.join(backuptool::manifest::MANIFEST_NAME));
            let needs_pw = man.as_ref().map(|m| m.cipher != "none").unwrap_or(false);
            let pass = ui.get_r_passphrase().to_string();
            if needs_pw && pass.is_empty() { ui.set_status(tr("need_password").into()); return; }

            let target = { let t = ui.get_r_target().to_string(); if t.is_empty() { "/".to_string() } else { t } };
            let ropt = RestoreOptions {
                backup_dir: folder,
                set: Some(setname),
                target,
                workers: ui.get_r_workers().max(1) as usize,
                reapply_meta: ui.get_r_reapply(),
                dry_run: ui.get_r_dry(),
                passphrase: if needs_pw { Some(pass) } else { None },
            };
            ui.set_status(tr("running").into());
            spawn_job(&ui, move |progress, log| match engine::restore(&ropt, progress, log) {
                Ok(s) => format!("{} restored, {} errors.", s.copied, s.errors),
                Err(e) => format!("{e}"),
            });
        });
    }

    ui.run()?;
    Ok(())
}
