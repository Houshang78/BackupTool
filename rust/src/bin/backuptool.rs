// SPDX-License-Identifier: GPL-3.0-or-later
//! Command line (clap). Encryption is selectable via --cipher.

use anyhow::{anyhow, Result};
use backuptool::crypto::Cipher;
use backuptool::engine::{self, BackupOptions, RestoreOptions};
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};

#[derive(Parser)]
#[command(name = "backuptool", version, about = "Parallel, incremental backup (Rust)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)] // clap subcommands naturally vary in size
enum Cmd {
    /// Back up
    Backup {
        /// source folders/files
        sources: Vec<String>,
        /// auto-pick this OS's user-data folders (Documents, Desktop, … — no OS files)
        #[arg(long)]
        auto: bool,
        /// Windows: snapshot the volume first (VSS) so locked/open files copy too
        #[arg(long)]
        vss: bool,
        /// before backup, ask file-locking apps (browsers, Office, …) to close
        #[arg(long = "close-apps")]
        close_apps: bool,
        /// skip reading the destination back to confirm the copy (verify is on by default)
        #[arg(long = "no-verify")]
        no_verify: bool,
        /// destination (default: suggested folder on the app's own drive)
        #[arg(short, long)]
        dest: Option<String>,
        /// backup set name (default: hostname)
        #[arg(short, long)]
        set: Option<String>,
        /// workers (default: CPU cores)
        #[arg(short = 'j', long)]
        workers: Option<usize>,
        /// compare by BLAKE3 instead of mtime/size
        #[arg(short = 'c', long)]
        checksum: bool,
        /// exclude pattern (repeatable), e.g. -e '**/node_modules/**'
        #[arg(short = 'e', long)]
        exclude: Vec<String>,
        /// delete in destination what was removed from the source
        #[arg(long)]
        delete: bool,
        /// also back up system dirs (/etc, ...); automatic when run as root
        #[arg(long)]
        system: bool,
        /// do not include system dirs even when running as root
        #[arg(long = "no-system")]
        no_system: bool,
        #[arg(short = 'n', long = "dry-run")]
        dry_run: bool,
        /// encryption: none | aes256gcm | chacha20poly1305
        #[arg(long, default_value = "none")]
        cipher: String,
        /// also back up this UID/username's home/data dir (repeatable)
        #[arg(long)]
        uid: Vec<String>,
        /// dump a database into the set: postgresql|mysql|redis|mongodb|all
        #[arg(long)]
        db: Vec<String>,
        /// generic DB dump (e.g. Oracle): NAME=command, runs with $BACKUPTOOL_DB_OUT
        #[arg(long = "db-command")]
        db_command: Vec<String>,
        #[arg(long = "db-host")]
        db_host: Option<String>,
        #[arg(long = "db-port")]
        db_port: Option<String>,
        #[arg(long = "db-user")]
        db_user: Option<String>,
        #[arg(long = "db-password")]
        db_password: Option<String>,
        #[arg(long = "db-socket")]
        db_socket: Option<String>,
    },
    /// Restore
    Restore {
        #[arg(short = 'S', long)]
        source: String,
        #[arg(short, long)]
        set: Option<String>,
        #[arg(short, long, default_value = "/")]
        target: String,
        #[arg(short = 'j', long)]
        workers: Option<usize>,
        /// do NOT reapply permissions/owner
        #[arg(long)]
        no_meta: bool,
        #[arg(short = 'n', long = "dry-run")]
        dry_run: bool,
    },
    /// List backup sets
    List {
        #[arg(short, long)]
        dest: String,
    },
    /// List auto-detected sources and destinations
    Discover,
    /// List databases that can be dumped
    Databases,
    /// Evacuate files to external storage (copy + verify, optional move), then
    /// print factory-reset guidance. Nothing is deleted unless every file is
    /// copied AND verified.
    Decommission {
        /// what to evacuate: home | config | disk
        #[arg(long, default_value = "home")]
        scope: String,
        /// for scope=config (or extra mount points for scope=disk): source paths
        sources: Vec<String>,
        /// external destination (default: suggested folder on the app's own drive);
        /// must be on a DIFFERENT device than the source
        #[arg(short, long)]
        dest: Option<String>,
        /// backup set name (default: <hostname>-decommission)
        #[arg(short, long)]
        set: Option<String>,
        /// workers (default: CPU cores)
        #[arg(short = 'j', long)]
        workers: Option<usize>,
        /// exclude pattern (repeatable)
        #[arg(short = 'e', long)]
        exclude: Vec<String>,
        /// encryption for the evacuated copy: none | aes256gcm | chacha20poly1305
        #[arg(long, default_value = "none")]
        cipher: String,
        /// after a verified copy, DELETE the source files (turns copy into move)
        #[arg(long = "delete-source")]
        delete_source: bool,
        /// securely overwrite source files before deleting (Phase 2; implies --delete-source)
        #[arg(long = "secure-wipe")]
        secure_wipe: bool,
        /// overwrite passes for --secure-wipe
        #[arg(long = "wipe-passes", default_value_t = 1)]
        wipe_passes: u32,
        /// after deletion, restore this backup folder as factory-default config
        /// (Phase 2; implies --delete-source)
        #[arg(long = "restore-defaults")]
        restore_defaults: Option<String>,
        /// set name inside --restore-defaults (default: only set present)
        #[arg(long = "defaults-set")]
        defaults_set: Option<String>,
        /// where to restore the defaults (default: /)
        #[arg(long = "defaults-target", default_value = "/")]
        defaults_target: String,
        /// skip the interactive 'ERASE' confirmation for --delete-source (for automation)
        #[arg(long)]
        force: bool,
        /// skip the different-device safety check (discouraged; testing/edge only)
        #[arg(long = "allow-same-device")]
        allow_same_device: bool,
        /// Phase 3: after evacuation, run the platform factory reset (Windows) or
        /// print manual instructions (macOS/Linux)
        #[arg(long = "reset-os")]
        reset_os: bool,
        /// Phase 3: custom reset command to run after evacuation (appliances)
        #[arg(long = "reset-command")]
        reset_command: Option<String>,
        #[arg(short = 'n', long = "dry-run")]
        dry_run: bool,
    },
    /// Clone a disk/partition to an image or device using a detected tool (Phase 3)
    Clone {
        /// list detected clone tools on this system and exit
        #[arg(long = "list-tools")]
        list_tools: bool,
        /// clone tool to use (e.g. dd, ddrescue); see --list-tools
        #[arg(long)]
        tool: Option<String>,
        /// source device or image file
        #[arg(long)]
        source: Option<String>,
        /// target device or image file (WILL BE OVERWRITTEN)
        #[arg(long)]
        target: Option<String>,
        /// skip the typed confirmation (for automation)
        #[arg(long)]
        force: bool,
        /// print the command instead of running it
        #[arg(short = 'n', long = "dry-run")]
        dry_run: bool,
    },
}

fn default_workers() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
}

fn is_root() -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn ask_passphrase(confirm: bool) -> Result<String> {
    // For automation/tests: read the password from an environment variable,
    // otherwise prompt interactively.
    if let Ok(p) = std::env::var("BACKUPTOOL_PASSWORD") {
        if !p.is_empty() {
            return Ok(p);
        }
    }
    let p = rpassword::prompt_password("Password: ")?;
    if confirm {
        let p2 = rpassword::prompt_password("Repeat password: ")?;
        if p != p2 {
            return Err(anyhow!("Passwords do not match"));
        }
    }
    Ok(p)
}

fn make_bar() -> ProgressBar {
    let bar = ProgressBar::new(0);
    bar.set_style(
        ProgressStyle::with_template("  {bar:40} {pos}/{len} ({percent}%) {msg}")
            .unwrap(),
    );
    bar
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Backup { sources, auto, vss, close_apps, no_verify, dest, set, workers, checksum, exclude, delete, system, no_system, dry_run, cipher,
                      uid, db, db_command, db_host, db_port, db_user, db_password, db_socket } => {
            let mut sources = sources;
            if auto {
                sources = engine::auto_sources();
                if sources.is_empty() {
                    return Err(anyhow!("--auto found no user-data folders; pass sources explicitly."));
                }
                println!("Auto sources: {}", sources.join(", "));
            }
            if sources.is_empty() {
                return Err(anyhow!("Provide at least one source (or use --auto)."));
            }
            let cipher = Cipher::parse(&cipher)?;
            let passphrase = if cipher != Cipher::None { Some(ask_passphrase(true)?) } else { None };
            let dest = dest.unwrap_or_else(|| {
                let d = engine::suggested_dest();
                println!("Using suggested destination: {d}");
                d
            });
            let set = set.unwrap_or_else(|| gethostname::gethostname().to_string_lossy().into_owned());
            let include_system = if no_system { false } else { system || is_root() };

            for u in &uid {                       // add other users' / service data dirs
                match backuptool::discover::resolve_uid(u) {
                    Some(d) => { println!("UID {u} -> {d}"); if !sources.contains(&d) { sources.push(d); } }
                    None => println!("UID {u}: no home/data dir found"),
                }
            }

            let (dest_for_db, set_for_db) = (dest.clone(), set.clone());

            // Warn about / close apps that hold files open.
            let blockers = backuptool::procs::running_blockers();
            if !blockers.is_empty() {
                println!("Apps that may lock files are running: {}", blockers.join(", "));
                if close_apps {
                    let closed = backuptool::procs::close_apps();
                    println!("Asked to close: {}", closed.join(", "));
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    let still = backuptool::procs::running_blockers();
                    if !still.is_empty() {
                        println!("Still running: {} — files they hold may still fail.", still.join(", "));
                    }
                } else {
                    println!("Close them, or use --close-apps / --vss, to avoid 'in use' errors.");
                }
            }

            // VSS (Windows): snapshot each source volume, read from the snapshot.
            let (scan_sources, vss_map, shadows) = if vss && !dry_run {
                match backuptool::vss::prepare(&sources) {
                    Ok(v) => {
                        for (dev, vol) in &v.1 { println!("VSS snapshot of {vol}: {}", dev.trim_end_matches('\\')); }
                        v
                    }
                    Err(e) => { println!("VSS unavailable ({e}); continuing without snapshot."); (sources.clone(), vec![], vec![]) }
                }
            } else {
                if vss { println!("--vss ignored in dry-run."); }
                (sources.clone(), vec![], vec![])
            };

            let opt = BackupOptions {
                sources: scan_sources, dest, set,
                workers: workers.unwrap_or_else(default_workers),
                use_checksum: checksum, excludes: exclude, prune: delete,
                include_system, dry_run, cipher, passphrase, vss_map,
                verify: !no_verify,
            };
            let bar = make_bar();
            let log = |m: &str| { bar.suspend(|| println!("{m}")); };
            let prog = |d: u64, t: u64| { bar.set_length(t); bar.set_position(d); };
            let stats = engine::backup(&opt, prog, log);
            bar.finish_and_clear();
            for sh in &shadows { backuptool::vss::remove(&sh.id); }  // always clean up snapshots
            let stats = stats?;

            if !dry_run && (!db.is_empty() || !db_command.is_empty()) {
                use backuptool::dbdump;
                let conn = dbdump::DbConn {
                    host: db_host, port: db_port, user: db_user,
                    password: db_password, socket: db_socket,
                };
                let mut specs: Vec<dbdump::DbSpec> = Vec::new();
                if !db.is_empty() {
                    let detected = dbdump::detect_databases();
                    if db.iter().any(|d| d == "all") {
                        specs.extend(detected.into_iter().filter(|d| d.running));  // running only
                    } else {
                        for want in &db {
                            specs.extend(detected.iter().filter(|d| &d.name == want || &d.kind == want).cloned());
                        }
                    }
                }
                for item in &db_command {
                    if let Some((n, c)) = item.split_once('=') {
                        specs.push(dbdump::DbSpec { name: n.into(), kind: "generic".into(), shell: Some(c.into()), running: true });
                    }
                }
                if !specs.is_empty() {
                    let out_dir = std::path::Path::new(&dest_for_db).join(&set_for_db).join(dbdump::DB_DIR);
                    println!("Dumping {} database(s) -> {}", specs.len(), out_dir.display());
                    dbdump::dump_all(&specs, &out_dir, &conn, &|m: &str| println!("{m}"));
                }
            }
            if stats.errors > 0 { std::process::exit(1); }
        }
        Cmd::Restore { source, set, target, workers, no_meta, dry_run } => {
            // Detect encryption from the manifest first -> ask for the password if needed.
            let probe_root = match &set {
                Some(s) => std::path::Path::new(&source).join(s),
                None => std::path::PathBuf::from(&source),
            };
            let man = backuptool::manifest::load(&probe_root.join(backuptool::manifest::MANIFEST_NAME));
            let needs_pw = man.as_ref().map(|m| m.cipher != "none").unwrap_or(false);
            let passphrase = if needs_pw { Some(ask_passphrase(false)?) } else { None };
            let opt = RestoreOptions {
                backup_dir: source, set, target,
                workers: workers.unwrap_or_else(default_workers),
                reapply_meta: !no_meta, dry_run, passphrase,
            };
            let bar = make_bar();
            let log = |m: &str| { bar.suspend(|| println!("{m}")); };
            let prog = |d: u64, t: u64| { bar.set_length(t); bar.set_position(d); };
            let stats = engine::restore(&opt, prog, log)?;
            bar.finish_and_clear();
            if stats.errors > 0 { std::process::exit(1); }
        }
        Cmd::Discover => {
            use backuptool::discover;
            println!("Sources (auto-detected):");
            for c in discover::user_data_sources() {
                println!("  {:8} {}", c.kind, c.path);
            }
            println!("Destinations (removable / network):");
            let dests = discover::detect_destinations();
            if dests.is_empty() {
                println!("  (none mounted)");
            }
            for c in dests {
                println!("  {:8} {}", c.kind, c.path);
            }
            println!("Suggested destination: {}",
                discover::default_destination().unwrap_or_else(|| "(none)".into()));
        }
        Cmd::Databases => {
            let dbs = backuptool::dbdump::detect_databases();
            if dbs.is_empty() {
                println!("No supported databases detected.");
            } else {
                println!("Detected databases:");
                for d in dbs {
                    println!("  {:12} ({})  {}", d.kind, d.name, if d.running { "running" } else { "stopped" });
                }
                println!("Dump with:       backuptool backup ... --db all   (running ones)");
                println!("Generic/Oracle:  backuptool backup ... --db-command 'oracle=expdp ...'");
            }
        }
        Cmd::List { dest } => {
            let sets = engine::list_sets(&dest);
            if sets.is_empty() {
                println!("No backup sets in {dest}");
            } else {
                println!("{:24} {:18} {:20} FILES", "SET", "HOST", "CREATED");
                for (s, h, c, n) in sets {
                    println!("{s:24} {h:18} {c:20} {n}");
                }
            }
        }
        Cmd::Decommission { scope, sources, dest, set, workers, exclude, cipher,
                            delete_source, secure_wipe, wipe_passes, restore_defaults,
                            defaults_set, defaults_target, force, allow_same_device,
                            reset_os, reset_command, dry_run } => {
            let cipher = Cipher::parse(&cipher)?;
            let resolved = engine::scope_sources(&scope, &sources)?;
            let mut excludes = exclude;
            if scope == "disk" {
                excludes.extend(engine::disk_excludes());
            }
            let dest = dest.unwrap_or_else(|| {
                let d = engine::suggested_dest();
                println!("Using suggested destination: {d}");
                d
            });
            // Phase 2 reset actions both require clearing the source first.
            let delete_source = delete_source || secure_wipe || restore_defaults.is_some();
            let passphrase = if cipher != Cipher::None { Some(ask_passphrase(true)?) } else { None };
            let set = set.unwrap_or_else(|| {
                format!("{}-decommission", gethostname::gethostname().to_string_lossy())
            });

            // Describe what the reset step will do (for the warning + final note).
            let action = if let Some(d) = &restore_defaults {
                format!("wipe user data, then restore factory defaults from '{d}'")
            } else if secure_wipe {
                format!("securely overwrite ({} pass(es)) and delete the source files", wipe_passes.max(1))
            } else if delete_source {
                "delete the source files".to_string()
            } else {
                "keep the source files (copy only)".to_string()
            };

            // Warning + typed confirmation before any source deletion.
            if delete_source && !dry_run && !force {
                println!("\n⚠️  WARNING — after a VERIFIED copy to '{dest}', this will:");
                println!("        {action}");
                println!("    in the ORIGINAL location(s):");
                for s in &resolved {
                    println!("        {s}");
                }
                println!("    This is PERMANENT. (Nothing happens unless every file is copied AND BLAKE3-verified.)");
                print!("\n  Type 'ERASE' to confirm, anything else to abort: ");
                std::io::Write::flush(&mut std::io::stdout()).ok();
                let mut line = String::new();
                std::io::stdin().read_line(&mut line)?;
                if line.trim() != "ERASE" {
                    return Err(anyhow!("Aborted — confirmation 'ERASE' not given."));
                }
            }

            let opt = engine::EvacuateOptions {
                sources: resolved,
                dest,
                set,
                workers: workers.unwrap_or_else(default_workers),
                excludes,
                cipher,
                passphrase,
                delete_source,
                secure_wipe,
                wipe_passes,
                allow_same_device,
                dry_run,
            };
            let bar = make_bar();
            let log = |m: &str| { bar.suspend(|| println!("{m}")); };
            let prog = |d: u64, t: u64| { bar.set_length(t); bar.set_position(d); };
            let report = engine::evacuate(&opt, prog, log)?;
            bar.finish_and_clear();
            println!(
                "Evacuation: {} scanned, {} copied, {} verified, {} deleted ({} securely wiped, {} delete errors).",
                report.scanned, report.copied, report.verified, report.deleted, report.wiped, report.delete_errors
            );

            // Phase 2 action (b): restore a factory-default configuration after wiping.
            let mut defaults_errors = 0u64;
            if let Some(def_dir) = restore_defaults {
                if dry_run {
                    println!("[dry-run] would restore defaults from '{def_dir}' -> '{defaults_target}'.");
                } else if report.delete_errors == 0 {
                    println!("Restoring factory-default config from '{def_dir}' ...");
                    let probe = match &defaults_set {
                        Some(s) => std::path::Path::new(&def_dir).join(s),
                        None => std::path::PathBuf::from(&def_dir),
                    };
                    let dman = backuptool::manifest::load(&probe.join(backuptool::manifest::MANIFEST_NAME));
                    let needs_pw = dman.as_ref().map(|m| m.cipher != "none").unwrap_or(false);
                    let def_pw = if needs_pw { Some(ask_passphrase(false)?) } else { None };
                    let ropt = RestoreOptions {
                        backup_dir: def_dir,
                        set: defaults_set,
                        target: defaults_target,
                        workers: workers.unwrap_or_else(default_workers),
                        reapply_meta: true,
                        dry_run: false,
                        passphrase: def_pw,
                    };
                    let bar = make_bar();
                    let log = |m: &str| { bar.suspend(|| println!("{m}")); };
                    let prog = |d: u64, t: u64| { bar.set_length(t); bar.set_position(d); };
                    let dstats = engine::restore(&ropt, prog, log)?;
                    bar.finish_and_clear();
                    defaults_errors = dstats.errors;
                    println!("Defaults restored: {} files, {} errors.", dstats.copied, dstats.errors);
                } else {
                    println!("Skipping defaults restore: source deletion had errors.");
                    defaults_errors = 1;
                }
            }

            // Phase 3 action (c): run the platform factory reset / a custom command.
            println!("\n--- Factory reset ---");
            let reset_argv: Option<Vec<String>> = if let Some(cmd) = &reset_command {
                Some(vec!["/bin/sh".into(), "-c".into(), cmd.clone()])
            } else if reset_os {
                backuptool::reset::factory_reset_command()
            } else {
                None
            };
            if let Some(argv) = reset_argv {
                if dry_run {
                    println!("[dry-run] would run reset command: {}", argv.join(" "));
                } else {
                    println!("Running reset command: {}", argv.join(" "));
                    let code = backuptool::reset::run_command(&argv)?;
                    println!("Reset command exited with code {code}.");
                }
            } else if reset_os || reset_command.is_some() {
                // reset requested but not automatable on this platform
                println!("No automatic reset on this platform — do it manually:");
                println!("{}", backuptool::reset::instructions());
            } else {
                println!("{}", backuptool::reset::instructions());
            }

            if report.verify_errors > 0 || report.backup_errors > 0
                || report.delete_errors > 0 || defaults_errors > 0 {
                std::process::exit(1);
            }
        }
        Cmd::Clone { list_tools, tool, source, target, force, dry_run } => {
            use backuptool::reset;
            if list_tools {
                println!("Clone tools on this system:");
                for t in reset::detect_clone_tools() {
                    let mark = if t.available() { "✓" } else { "—" };
                    let loc = t.path.as_deref().unwrap_or("not installed");
                    println!("  [{mark}] {:<14} {}  ({loc})", t.name, t.description);
                }
                return Ok(());
            }
            let tool = tool.ok_or_else(|| anyhow!("--tool is required (see --list-tools)"))?;
            let source = source.ok_or_else(|| anyhow!("--source is required"))?;
            let target = target.ok_or_else(|| anyhow!("--target is required"))?;

            // Refuse if the chosen tool isn't installed.
            let installed = reset::detect_clone_tools().into_iter()
                .find(|t| t.name == tool).map(|t| t.available()).unwrap_or(false);
            if !installed && reset::which(&tool).is_none() {
                println!("Clone tool '{tool}' is not installed on this system.");
                println!("Suggestion: install it, or pick an available one from --list-tools.");
                println!("On platforms without a CLI cloner, use Clonezilla, Macrium Reflect or\n\
                          'Disk Utility' (macOS) / 'dd' from a live USB instead.");
                std::process::exit(1);
            }

            let argv = reset::build_clone_command(&tool, &source, &target)?;
            if dry_run {
                println!("[dry-run] would run: {}", argv.join(" "));
                return Ok(());
            }
            if !force {
                println!("\n⚠️  WARNING — this will OVERWRITE the target '{target}' with a clone of '{source}'.");
                print!("  Type 'CLONE' to confirm, anything else to abort: ");
                std::io::Write::flush(&mut std::io::stdout()).ok();
                let mut line = String::new();
                std::io::stdin().read_line(&mut line)?;
                if line.trim() != "CLONE" {
                    return Err(anyhow!("Aborted — confirmation 'CLONE' not given."));
                }
            }
            println!("Running: {}", argv.join(" "));
            let code = reset::run_command(&argv)?;
            println!("Clone command exited with code {code}.");
            if code != 0 { std::process::exit(1); }
        }
    }
    Ok(())
}
