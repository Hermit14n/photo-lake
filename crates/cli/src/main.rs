use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use photolake_core::Config;
use photolake_ingest::{ImportOptions, Importer};
use photolake_lake::Lake;

#[derive(Parser)]
#[command(
    name = "photolake",
    about = "Personal photo lakehouse (Iceberg + object storage)"
)]
struct Cli {
    /// Config file (default: ~/.config/photolake/config.toml)
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Write a default config (if missing), create buckets and Iceberg tables.
    Init,
    /// Import photos from a folder or card (looks for DCIM/).
    Import {
        path: PathBuf,
        /// Skip bird detection.
        #[arg(long)]
        no_ml: bool,
    },
    /// Run bird detection on one image file and print the findings.
    Detect { path: PathBuf },
    /// Re-run bird ML over photos already in the library.
    Reprocess {
        /// Reprocess every photo, not just those the current models haven't seen.
        #[arg(long)]
        all: bool,
    },
    /// Run the web UI + API (and, later, the card watcher and backup scheduler).
    Serve,
    /// Back up to all available targets (or one by name).
    Backup {
        #[arg(long)]
        target: Option<String>,
    },
    /// Restore a backup folder into the object store + catalog.
    Restore {
        #[arg(long)]
        from: PathBuf,
        /// Replace an existing catalog.
        #[arg(long)]
        force: bool,
    },
    /// Merge small Iceberg data files (runs nightly automatically under `serve`).
    Compact {
        /// Only compact tables with more than this many data files.
        #[arg(long, default_value_t = 1)]
        min_files: usize,
    },
    /// List mounted drives (with filesystem UUIDs for backup targets).
    Drives,
    /// Run a SQL query against the lake and print JSON rows.
    Sql { query: String },
    /// Undo a schema upgrade: restore the catalog saved just before it (stop photolake first).
    Rollback {
        /// Snapshot file to restore (default: the newest). Use --list to see them.
        snapshot: Option<PathBuf>,
        #[arg(long)]
        list: bool,
    },
    /// Exit 0 if the server answers (used by the container health check).
    Health,
    /// Print the version of this build and of the library's schema.
    Version,
    /// Set the web UI password (reads it from the terminal, or from stdin when piped).
    Passwd {
        /// Set the read-only "viewer" password instead of the admin one.
        #[arg(long)]
        viewer: bool,
        /// Remove the password (only allowed while the UI listens on 127.0.0.1).
        #[arg(long)]
        clear: bool,
    },
}

fn read_password() -> Result<String> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        return Ok(line.trim_end_matches(['\r', '\n']).to_string());
    }
    let a = rpassword::prompt_password("New password: ")?;
    let b = rpassword::prompt_password("Repeat password: ")?;
    anyhow::ensure!(a == b, "passwords don't match");
    Ok(a)
}

fn load_analyzer(cfg: &Config) -> Result<Option<Arc<dyn photolake_ml::Analyzer>>> {
    if !cfg.ml.enabled {
        return Ok(None);
    }
    let dir = cfg.models_dir();
    if !dir.join("manifest.json").exists() {
        tracing::warn!(
            "bird models not found in {}; run tools/export_models.py (importing without ML)",
            dir.display()
        );
        return Ok(None);
    }
    let a = photolake_ml::BirdAnalyzer::load(&dir, &cfg.ml, &cfg.data_dir().join("cache"))?;
    Ok(Some(Arc::new(a)))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,iceberg=warn,datafusion=warn,sqlx=warn,nom_exif=error,rawler=warn,ort=warn".into()),
        )
        .init();
    let cli = Cli::parse();
    let (cfg, cfg_path) = Config::load(cli.config.as_deref())?;

    match cli.cmd {
        Cmd::Rollback { snapshot, list } => {
            use photolake_lake::migrate;
            let dir = cfg.data_dir().join("pre-upgrade");
            let snaps = migrate::list_snapshots(&dir);
            if list || snaps.is_empty() {
                if snaps.is_empty() {
                    println!("no pre-upgrade snapshots in {}", dir.display());
                }
                for s in &snaps {
                    println!("{}", s.display());
                }
                return Ok(());
            }
            let pick = snapshot.unwrap_or_else(|| snaps[0].clone());
            let kept = migrate::restore_snapshot(&pick, &cfg.catalog_db_path())?;
            println!(
                "restored {}\n(the replaced catalog is kept at {})\n\
                 Now start the Photolake version you were using before the upgrade.",
                pick.display(),
                kept.display()
            );
        }
        Cmd::Health => {
            use std::io::{Read, Write};
            let port = cfg.server.bind.rsplit(':').next().unwrap_or("7878");
            let addr = format!("127.0.0.1:{port}");
            let timeout = std::time::Duration::from_secs(4);
            let mut s = std::net::TcpStream::connect_timeout(&addr.parse()?, timeout)?;
            s.set_read_timeout(Some(timeout))?;
            write!(
                s,
                "GET /api/auth/me HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
            )?;
            let mut head = [0u8; 12];
            s.read_exact(&mut head)?;
            anyhow::ensure!(head.ends_with(b"200"), "server unhealthy");
        }
        Cmd::Version => {
            println!(
                "photolake {} (lake schema v{}, {})",
                env!("CARGO_PKG_VERSION"),
                photolake_lake::migrate::SCHEMA_VERSION,
                if cfg!(feature = "cuda") {
                    "GPU build"
                } else {
                    "CPU build"
                }
            );
        }
        Cmd::Passwd { viewer, clear } => {
            let path = cfg_path
                .clone()
                .ok_or_else(|| anyhow::anyhow!("no config location; pass --config"))?;
            let mut cfg = cfg;
            let hash = if clear {
                None
            } else {
                Some(photolake_server::auth::hash_password(&read_password()?)?)
            };
            let auth = &mut cfg.server.auth;
            if viewer {
                auth.viewer_password_hash = hash;
            } else {
                if hash.is_none() && auth.viewer_password_hash.is_some() {
                    anyhow::bail!("remove the viewer password first (--viewer --clear)");
                }
                auth.admin_password_hash = hash;
            }
            cfg.save(&path)?;
            // Sessions from the old password must not survive the change.
            let _ = std::fs::remove_file(cfg.data_dir().join("sessions.json"));
            println!(
                "{} password {} in {}. Restart photolake to apply (photolake-ctl restart).",
                if viewer { "viewer" } else { "admin" },
                if clear { "removed" } else { "saved" },
                path.display()
            );
        }
        Cmd::Init => {
            if let Some(p) = &cfg_path
                && !p.exists()
            {
                cfg.save(p)?;
                println!("wrote default config to {}", p.display());
            }
            photolake_core::store::ensure_buckets(&cfg.storage).await?;
            Lake::open(&cfg).await?;
            println!("lake ready (catalog: {})", cfg.catalog_db_path().display());
        }
        Cmd::Import { path, no_ml } => {
            photolake_core::store::ensure_buckets(&cfg.storage).await?;
            let lake = Arc::new(Lake::open(&cfg).await?);
            let analyzer = if no_ml { None } else { load_analyzer(&cfg)? };
            let importer = Importer::new(cfg, lake, analyzer)?;
            let batch = importer
                .import_path(
                    &path,
                    ImportOptions {
                        source_fs_uuid: None,
                    },
                )
                .await?;
            println!("{}", serde_json::to_string_pretty(&batch)?);
        }
        Cmd::Detect { path } => {
            let analyzer = load_analyzer(&cfg)?.ok_or_else(|| anyhow::anyhow!("ML is disabled"))?;
            let clock = photolake_ingest::meta::CameraClock::from_config(None);
            let meta = photolake_ingest::meta::read_image_meta(&path, &clock).unwrap_or_default();
            let img =
                photolake_ingest::render::decode_jpeg(&std::fs::read(&path)?, meta.orientation)?;
            let t = std::time::Instant::now();
            let ctx = photolake_ml::AnalyzeContext {
                captured_at: meta.captured_at,
                lat: meta.gps_lat,
                lon: meta.gps_lon,
            };
            let findings = analyzer.analyze(&img, &ctx)?;
            println!("{} bird(s) in {:?}", findings.len(), t.elapsed());
            for (i, f) in findings.iter().enumerate() {
                println!(
                    "  #{i} det={:.2} box=({:.3},{:.3},{:.3},{:.3})",
                    f.det_score, f.bbox.x, f.bbox.y, f.bbox.w, f.bbox.h
                );
                for c in &f.candidates {
                    println!(
                        "      {:<32} {:<30} p={:.3} final={:.3}",
                        c.common_name, c.scientific_name, c.cls_score, c.final_score
                    );
                }
            }
        }
        Cmd::Reprocess { all } => {
            let lake = Lake::open(&cfg).await?;
            let analyzer = load_analyzer(&cfg)?.ok_or_else(|| anyhow::anyhow!("ML is disabled"))?;
            let scope = if all {
                photolake_ingest::reprocess::Scope::All
            } else {
                photolake_ingest::reprocess::Scope::Missing
            };
            let (n, birds) =
                photolake_ingest::reprocess::reprocess(&cfg, &lake, analyzer, scope, |d, t, b| {
                    if d % 25 == 0 || d == t {
                        eprintln!("  {d}/{t} processed, {b} with birds");
                    }
                })
                .await?;
            println!("reprocessed {n} photos, {birds} with birds");
        }
        Cmd::Serve => {
            photolake_core::store::ensure_buckets(&cfg.storage).await?;
            let lake = Arc::new(Lake::open(&cfg).await?);
            let analyzer = load_analyzer(&cfg)?;
            let importer = Arc::new(Importer::new(cfg.clone(), lake.clone(), analyzer)?);
            let app = photolake_server::build_state(photolake_server::ServerParts {
                cfg,
                cfg_path,
                lake,
                importer,
            })
            .await?;
            photolake_server::serve(app).await?;
        }
        Cmd::Backup { target } => {
            if cfg.backup.targets.is_empty() {
                anyhow::bail!(
                    "no backup targets configured (see [[backup.targets]] in the config)"
                );
            }
            let mgr = photolake_backup::BackupManager::new(&cfg);
            let progress = |p: photolake_backup::Progress| {
                if p.total > 0 {
                    eprintln!("  {} {}: {}/{}", p.target, p.phase, p.done, p.total);
                }
            };
            let results = match target {
                Some(name) => {
                    let t = cfg
                        .backup
                        .targets
                        .iter()
                        .find(|t| t.name == name)
                        .ok_or_else(|| anyhow::anyhow!("no target named {name}"))?;
                    vec![mgr.run(&cfg, t, progress).await]
                }
                None => mgr.run_all(&cfg, progress).await,
            };
            println!("{}", serde_json::to_string_pretty(&results)?);
            if results.iter().any(|r| !r.ok) {
                std::process::exit(1);
            }
        }
        Cmd::Restore { from, force } => {
            photolake_backup::restore_from_dir(&cfg, &from, force, |phase, d, t| {
                eprintln!("  {phase}: {d}/{t}")
            })
            .await?;
            let lake = Lake::open(&cfg).await?;
            let n = lake
                .query_json("SELECT COUNT(*) AS n FROM photos_current")
                .await?;
            println!("restored; {} photos in library", n[0]["n"]);
        }
        Cmd::Compact { min_files } => {
            let lake = Lake::open(&cfg).await?;
            for (t, b, a) in lake.compact_all(min_files).await? {
                println!("{t}: {b} -> {a} data files");
            }
        }
        Cmd::Drives => {
            for m in photolake_core::devices::list_mounts() {
                println!(
                    "{:<36} uuid={:<38} {:>8} free  {}{}",
                    m.mount_point.display(),
                    m.uuid.unwrap_or_default(),
                    m.free_bytes
                        .map(|b| format!("{:.0} GB", b as f64 / 1e9))
                        .unwrap_or_default(),
                    m.label.map(|l| format!("label={l} ")).unwrap_or_default(),
                    if m.is_camera_card {
                        "(camera card)"
                    } else {
                        ""
                    }
                );
            }
        }
        Cmd::Sql { query } => {
            let lake = Lake::open(&cfg).await?;
            let rows = lake.query_json(&query).await?;
            println!("{}", serde_json::to_string_pretty(&rows)?);
        }
    }
    Ok(())
}
