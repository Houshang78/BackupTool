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

/// Path for a timestamped GUI session log. Prefers the directory the app runs
/// from (portable: the log lands next to the executable, e.g. on the USB stick);
/// falls back to the home dir, then temp, if that location is read-only.
fn session_log_path() -> std::path::PathBuf {
    use std::path::PathBuf;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(p) = exe.parent() {
            candidates.push(p.to_path_buf());
        }
    }
    if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
        candidates.push(PathBuf::from(home));
    }
    candidates.push(std::env::temp_dir());
    for base in candidates {
        let dir = base.join("backuptool-logs");
        if std::fs::create_dir_all(&dir).is_ok() {
            // confirm the directory is actually writable before choosing it
            let probe = dir.join(".wtest");
            if std::fs::write(&probe, b"").is_ok() {
                let _ = std::fs::remove_file(&probe);
                return dir.join(format!("backuptool-gui-{stamp}.log"));
            }
        }
    }
    std::env::temp_dir().join(format!("backuptool-gui-{stamp}.log"))
}

/// Spawn a background job that reports progress/log to the UI and a final message.
/// Every log line is also appended to a timestamped session log file so the run
/// can be read back later.
fn spawn_job<J>(ui: &MainWindow, job: J)
where
    J: FnOnce(Prog, Log) -> String + Send + 'static,
{
    ui.set_running(true);
    ui.set_progress(0.0);

    // Open the session log file and announce its path in the log view.
    let logpath = session_log_path();
    let header = format!("Log file: {}", logpath.display());
    ui.set_logtext(format!("{header}\n").into());
    let logfile: Arc<Mutex<Option<std::fs::File>>> = Arc::new(Mutex::new(
        std::fs::OpenOptions::new().create(true).append(true).open(&logpath).ok(),
    ));
    if let Ok(mut g) = logfile.lock() {
        if let Some(f) = g.as_mut() {
            use std::io::Write;
            let _ = writeln!(f, "{header}");
        }
    }

    let handle: Handle = Arc::new(Mutex::new(ui.as_weak()));
    let (h_prog, h_log, h_fin) = (handle.clone(), handle.clone(), handle.clone());
    let lf = logfile.clone();

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
            // Append to the session log file.
            if let Ok(mut g) = lf.lock() {
                if let Some(f) = g.as_mut() {
                    use std::io::Write;
                    let _ = writeln!(f, "{line}");
                }
            }
            ui_apply(&h_log, move |ui| {
                let mut t = ui.get_logtext().to_string();
                t.push_str(&line);
                t.push('\n');
                ui.set_logtext(t.into());
            });
        });
        // Catch panics in the engine so a failure reports an error instead of
        // killing the worker thread (and leaving the UI stuck on "running").
        let msg = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(progress, log))) {
            Ok(m) => m,
            Err(_) => "Error: the operation crashed (panic) — see the log file for details.".to_string(),
        };
        ui_apply(&h_fin, move |ui| {
            ui.set_running(false);
            ui.set_status(msg.into());
        });
    });
}

fn nonempty(s: SharedString) -> Option<String> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
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
    ui.set_t_auto(t("auto_sources").into());
    ui.set_t_extra(t("extra_paths").into());
    ui.set_t_uid(t("uid_label").into());
    ui.set_t_db_group(t("db_group").into());
    ui.set_t_db_dump(t("db_dump").into());
    ui.set_t_db_cmd(t("db_cmd_label").into());
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
    ui.set_t_vss(t("opt_vss").into());
    // restore tab
    ui.set_t_tab_restore(t("tab_restore").into());
    ui.set_t_folder(t("backup_folder").into());
    ui.set_t_rset(t("backup_set").into());
    ui.set_t_loadsets(t("load_sets").into());
    ui.set_t_target(t("target_root").into());
    ui.set_t_reapply(t("reapply_meta").into());
    ui.set_t_start_restore(t("start_restore").into());
    // decommission tab
    ui.set_t_tab_decommission(t("tab_decommission").into());
    ui.set_t_scope(t("scope").into());
    ui.set_t_reset_action(t("reset_action").into());
    ui.set_t_secure_wipe(t("secure_wipe").into());
    ui.set_t_wipe_passes(t("wipe_passes").into());
    ui.set_t_defaults_dir(t("defaults_dir").into());
    ui.set_t_defaults_set(t("defaults_set").into());
    ui.set_t_defaults_target(t("defaults_target").into());
    ui.set_t_allow_same_device(t("allow_same_device").into());
    ui.set_t_confirm_erase(t("confirm_erase").into());
    ui.set_t_start_decommission(t("start_decommission").into());
    // clone tab
    ui.set_t_tab_clone(t("tab_clone").into());
    ui.set_t_clone_tool(t("clone_tool").into());
    ui.set_t_clone_source(t("clone_source").into());
    ui.set_t_clone_target(t("clone_target").into());
    ui.set_t_list_tools(t("list_tools").into());
    ui.set_t_start_clone(t("start_clone").into());
    ui.set_t_confirm_clone(t("confirm_clone").into());
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
    if let Some(d) = backuptool::discover::default_destination() {
        ui.set_dest(d.into());   // USB / external / network if one is mounted
    }
    let cores = std::thread::available_parallelism().map(|n| n.get() as i32).unwrap_or(4);
    ui.set_workers(cores);
    ui.set_r_workers(cores);

    // Suggest a destination on the drive the app runs from (dated, per-user).
    let suggested = engine::suggested_dest();
    ui.set_dest(suggested.clone().into());
    ui.set_d_dest(suggested.into());

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

    // --- auto-detect sources (user/service/data dirs) ---
    {
        let weak = ui.as_weak();
        ui.on_auto_sources(move || {
            if let Some(ui) = weak.upgrade() {
                let mut out = ui.get_sources().to_string();
                let mut seen: std::collections::HashSet<String> =
                    out.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                for c in backuptool::discover::user_data_sources() {
                    if matches!(c.kind.as_str(), "user" | "service" | "data") && seen.insert(c.path.clone()) {
                        if !out.is_empty() {
                            out.push(';');
                        }
                        out.push_str(&c.path);
                    }
                }
                ui.set_sources(out.into());
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
        ui.on_auto_sources(move || {
            if let Some(ui) = weak.upgrade() {
                let s = engine::auto_sources().join(";");
                ui.set_sources(s.into());
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
            let mut sources: Vec<String> = ui.get_sources().split(';')
                .map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            // Merge the "extra paths/files" field (comma or semicolon separated).
            for p in ui.get_extra().split([';', ',']).map(|s| s.trim()).filter(|s| !s.is_empty()) {
                if !sources.iter().any(|s| s == p) {
                    sources.push(p.to_string());
                }
            }
            // Resolve UID(s)/usernames to their home/data dirs.
            for u in ui.get_uid().split([';', ',']).map(|s| s.trim()).filter(|s| !s.is_empty()) {
                if let Some(d) = backuptool::discover::resolve_uid(u) {
                    if !sources.iter().any(|s| s == &d) {
                        sources.push(d);
                    }
                }
            }
            if sources.is_empty() { ui.set_status(tr("need_source").into()); return; }
            let dest = ui.get_dest().to_string();
            if dest.is_empty() { ui.set_status(tr("need_dest").into()); return; }

            // Overlap check: warn about sources already covered by another.
            let overlaps = backuptool::discover::analyze_overlaps(&sources);
            if !overlaps.is_empty() {
                let mut desc = String::new();
                for o in overlaps.iter().take(20) {
                    desc.push_str(&format!("• {}  ⊂  {}\n", o.path, o.covered_by));
                }
                let answer = rfd::MessageDialog::new()
                    .set_level(rfd::MessageLevel::Warning)
                    .set_title(tr("overlap_title"))
                    .set_description(format!("{}\n\n{}\n[{} = Yes, {} = No]",
                        tr("overlap_msg"), desc, tr("overlap_remove"), tr("overlap_keep")))
                    .set_buttons(rfd::MessageButtons::YesNoCancel)
                    .show();
                match answer {
                    rfd::MessageDialogResult::Yes => {
                        let redundant: std::collections::HashSet<&str> =
                            overlaps.iter().map(|o| o.path.as_str()).collect();
                        sources.retain(|s| !redundant.contains(s.as_str()));
                    }
                    rfd::MessageDialogResult::No => {}            // keep all
                    _ => { ui.set_status(tr("ready").into()); return; }   // cancel
                }
            }

            // Database dump options (read on the UI thread).
            let set_name = ui.get_setname().to_string();
            let db_out = std::path::Path::new(&dest).join(&set_name).join(backuptool::dbdump::DB_DIR);
            let db_conn = backuptool::dbdump::DbConn {
                host: nonempty(ui.get_db_host()), port: nonempty(ui.get_db_port()),
                user: nonempty(ui.get_db_user()), password: nonempty(ui.get_db_password()), socket: None,
            };
            let mut db_specs: Vec<backuptool::dbdump::DbSpec> = Vec::new();
            if ui.get_db_dump() {
                db_specs.extend(backuptool::dbdump::detect_databases().into_iter().filter(|d| d.running));
            }
            if let Some((n, c)) = ui.get_db_command().to_string().split_once('=') {
                db_specs.push(backuptool::dbdump::DbSpec {
                    name: n.into(), kind: "generic".into(), shell: Some(c.into()), running: true,
                });
            }

            let opt = BackupOptions {
                sources,
                dest,
                set: set_name,
                workers: ui.get_workers().max(1) as usize,
                use_checksum: ui.get_checksum(),
                excludes: vec![],
                prune: ui.get_delete_missing(),
                include_system: ui.get_system(),
                dry_run: false,
                cipher,
                passphrase: if cipher != Cipher::None { Some(pass) } else { None },
                vss_map: Vec::new(),
                verify: true,
            };
            // Warn / offer to close file-locking apps before starting.
            let blockers = backuptool::procs::running_blockers();
            if !blockers.is_empty() {
                let choice = rfd::MessageDialog::new()
                    .set_title(tr("apps_title"))
                    .set_description(format!("{}\n\n{}\n\n{}",
                        tr("apps_msg"), blockers.join(", "), tr("apps_buttons")))
                    .set_buttons(rfd::MessageButtons::YesNoCancel)
                    .show();
                match choice {
                    rfd::MessageDialogResult::Yes => {
                        backuptool::procs::close_apps();
                        std::thread::sleep(std::time::Duration::from_secs(2));
                    }
                    rfd::MessageDialogResult::No => {}
                    _ => { ui.set_status(tr("ready").into()); return; }
                }
            }

            let do_vss = ui.get_vss();
            ui.set_status(tr("running").into());
            spawn_job(&ui, move |progress, log| {
                let mut opt = opt;
                let mut shadow_ids: Vec<String> = Vec::new();
                if do_vss {
                    match backuptool::vss::prepare(&opt.sources) {
                        Ok((src, map, shadows)) => {
                            for (dev, vol) in &map {
                                log(&format!("VSS snapshot of {vol}: {}", dev.trim_end_matches('\\')));
                            }
                            opt.sources = src;
                            opt.vss_map = map;
                            shadow_ids = shadows.into_iter().map(|s| s.id).collect();
                        }
                        Err(e) => log(&format!("VSS unavailable ({e}); continuing without snapshot.")),
                    }
                }
                let result = engine::backup(&opt, &progress, &log);
                for id in &shadow_ids {
                    backuptool::vss::remove(id);
                }
                if !db_specs.is_empty() {
                    log(&format!("Dumping {} database(s) -> {}", db_specs.len(), db_out.display()));
                    backuptool::dbdump::dump_all(&db_specs, &db_out, &db_conn, &log);
                }
                match result {
                    Ok(s) => format!("{} copied, {} skipped, {} errors.", s.copied, s.skipped, s.errors),
                    Err(e) => format!("{e}"),
                }
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

    // --- decommission tab pickers ---
    {
        let weak = ui.as_weak();
        ui.on_pick_d_dest(move || {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                if let Some(ui) = weak.upgrade() {
                    ui.set_d_dest(dir.to_string_lossy().into_owned().into());
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_pick_d_defaults(move || {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                if let Some(ui) = weak.upgrade() {
                    ui.set_d_defaults_dir(dir.to_string_lossy().into_owned().into());
                }
            }
        });
    }

    // --- decommission ---
    {
        let weak = ui.as_weak();
        let i18n = i18n.clone();
        let current = current.clone();
        ui.on_start_decommission(move || {
            let ui = match weak.upgrade() { Some(u) => u, None => return };
            let code = current.borrow().clone();
            let tr = |k: &str| i18n.tr(&code, k);

            let scope = ui.get_d_scope().to_string();
            let action = ui.get_d_reset_action().to_string();
            let secure_wipe = ui.get_d_secure_wipe();
            let dry = ui.get_d_dry();
            // Any deletion (delete / restore-defaults / secure-overwrite) is destructive.
            let delete_source = action == "delete" || action == "restore-defaults" || secure_wipe;
            if delete_source && !dry && !ui.get_d_confirm() {
                ui.set_status(tr("need_confirm").into());
                return;
            }
            let cipher = match Cipher::parse(&ui.get_d_cipher()) {
                Ok(c) => c,
                Err(e) => { ui.set_status(format!("{}: {e}", tr("error")).into()); return; }
            };
            let pass = ui.get_d_passphrase().to_string();
            if cipher != Cipher::None && pass.is_empty() {
                ui.set_status(tr("need_password").into());
                return;
            }
            let explicit: Vec<String> = ui.get_d_sources().split(';')
                .map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            let resolved = match engine::scope_sources(&scope, &explicit) {
                Ok(v) => v,
                Err(e) => { ui.set_status(format!("{}: {e}", tr("error")).into()); return; }
            };
            let dest = ui.get_d_dest().to_string();
            if dest.is_empty() { ui.set_status(tr("need_dest").into()); return; }

            let mut excludes: Vec<String> = vec![];
            if scope == "disk" { excludes.extend(engine::disk_excludes()); }

            let set = {
                let s = ui.get_d_set().to_string();
                if s.is_empty() {
                    format!("{}-decommission", gethostname::gethostname().to_string_lossy())
                } else { s }
            };
            let defaults = if action == "restore-defaults" {
                let d = ui.get_d_defaults_dir().to_string();
                if d.is_empty() {
                    ui.set_status(format!("{}: defaults folder", tr("missing")).into());
                    return;
                }
                let dset = ui.get_d_defaults_set().to_string();
                let dtgt = { let t = ui.get_d_defaults_target().to_string(); if t.is_empty() { "/".into() } else { t } };
                Some((d, if dset.is_empty() { None } else { Some(dset) }, dtgt))
            } else { None };

            let opt = engine::EvacuateOptions {
                sources: resolved, dest, set,
                workers: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4),
                excludes, cipher,
                passphrase: if cipher != Cipher::None { Some(pass.clone()) } else { None },
                delete_source, secure_wipe,
                wipe_passes: ui.get_d_wipe_passes().max(1) as u32,
                allow_same_device: ui.get_d_allow_same_device(),
                dry_run: dry,
            };
            let def_pass = if cipher != Cipher::None { Some(pass) } else { None };
            ui.set_status(tr("running").into());
            spawn_job(&ui, move |progress, log| {
                match engine::evacuate(&opt, &progress, &log) {
                    Ok(rep) => {
                        // Phase 2 action (b): restore defaults after a clean wipe.
                        if let Some((ddir, dset, dtgt)) = defaults {
                            if !opt.dry_run && rep.delete_errors == 0 {
                                let ropt = RestoreOptions {
                                    backup_dir: ddir, set: dset, target: dtgt,
                                    workers: opt.workers, reapply_meta: true,
                                    dry_run: false, passphrase: def_pass,
                                };
                                let _ = engine::restore(&ropt, &progress, &log);
                            }
                        }
                        log(&backuptool::reset::instructions());
                        format!("{} copied, {} verified, {} deleted ({} wiped, {} errors).",
                            rep.copied, rep.verified, rep.deleted, rep.wiped, rep.delete_errors)
                    }
                    Err(e) => format!("{e}"),
                }
            });
        });
    }

    // --- clone: detect tools ---
    {
        let weak = ui.as_weak();
        ui.on_list_clone_tools(move || {
            if let Some(ui) = weak.upgrade() {
                let tools = backuptool::reset::detect_clone_tools();
                let avail: Vec<SharedString> = tools.iter()
                    .filter(|t| t.available()).map(|t| t.name.into()).collect();
                let summary: String = tools.iter()
                    .map(|t| format!("[{}] {}", if t.available() { "✓" } else { "—" }, t.name))
                    .collect::<Vec<_>>().join("  ");
                if !avail.is_empty() { ui.set_c_tool(avail[0].clone()); }
                ui.set_c_tools(ModelRc::new(VecModel::from(avail)));
                ui.set_status(summary.into());
            }
        });
    }

    // --- clone: run ---
    {
        let weak = ui.as_weak();
        let i18n = i18n.clone();
        let current = current.clone();
        ui.on_start_clone(move || {
            let ui = match weak.upgrade() { Some(u) => u, None => return };
            let code = current.borrow().clone();
            let tr = |k: &str| i18n.tr(&code, k);

            let tool = ui.get_c_tool().to_string();
            let source = ui.get_c_source().to_string();
            let target = ui.get_c_target().to_string();
            let dry = ui.get_c_dry();
            if tool.is_empty() || source.is_empty() || target.is_empty() {
                ui.set_status(tr("need_target").into());
                return;
            }
            if !dry && !ui.get_c_confirm() {
                ui.set_status(tr("need_confirm").into());
                return;
            }
            let argv = match backuptool::reset::build_clone_command(&tool, &source, &target) {
                Ok(v) => v,
                Err(e) => { ui.set_status(format!("{}: {e}", tr("error")).into()); return; }
            };
            ui.set_status(tr("running").into());
            spawn_job(&ui, move |_progress, log| {
                if dry {
                    log(&format!("[dry-run] would run: {}", argv.join(" ")));
                    return "dry-run".to_string();
                }
                log(&format!("Running: {}", argv.join(" ")));
                match backuptool::reset::run_command(&argv) {
                    Ok(c) => format!("clone exited with code {c}"),
                    Err(e) => format!("{e}"),
                }
            });
        });
    }

    ui.run()?;
    Ok(())
}
