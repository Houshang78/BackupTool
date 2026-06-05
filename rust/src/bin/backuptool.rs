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
enum Cmd {
    /// Back up
    Backup {
        /// source folders/files
        sources: Vec<String>,
        #[arg(short, long)]
        dest: String,
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
        Cmd::Backup { sources, dest, set, workers, checksum, exclude, delete, system, no_system, dry_run, cipher } => {
            if sources.is_empty() {
                return Err(anyhow!("Provide at least one source."));
            }
            let cipher = Cipher::parse(&cipher)?;
            let passphrase = if cipher != Cipher::None { Some(ask_passphrase(true)?) } else { None };
            let set = set.unwrap_or_else(|| gethostname::gethostname().to_string_lossy().into_owned());
            let include_system = if no_system { false } else { system || is_root() };
            let opt = BackupOptions {
                sources, dest, set,
                workers: workers.unwrap_or_else(default_workers),
                use_checksum: checksum, excludes: exclude, prune: delete,
                include_system, dry_run, cipher, passphrase,
            };
            let bar = make_bar();
            let log = |m: &str| { bar.suspend(|| println!("{m}")); };
            let prog = |d: u64, t: u64| { bar.set_length(t); bar.set_position(d); };
            let stats = engine::backup(&opt, prog, log)?;
            bar.finish_and_clear();
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
    }
    Ok(())
}
