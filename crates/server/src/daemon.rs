//! Background work: card watcher, backup triggers and the daily backup schedule.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use photolake_backup::Progress;
use photolake_core::config::BackupTargetKind;
use photolake_core::devices;
use photolake_ingest::ImportEvent;

use crate::events::ServerEvent;
use crate::state::Shared;

const POLL: Duration = Duration::from_secs(2);

pub fn notify(summary: &str, body: &str) {
    let (s, b) = (summary.to_string(), body.to_string());
    // Best-effort; there may be no notification daemon (e.g. headless).
    std::thread::spawn(move || {
        let _ = notify_rust::Notification::new()
            .appname("Photolake")
            .summary(&s)
            .body(&b)
            .icon("camera-photo")
            .show();
    });
}

fn backup_uuids(app: &Shared) -> HashSet<String> {
    app.config()
        .backup
        .targets
        .iter()
        .filter_map(|t| match &t.kind {
            BackupTargetKind::Local {
                fs_uuid: Some(u), ..
            } if t.enabled => Some(u.to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

/// Watch for newly mounted filesystems. Cards present at startup are listed in the UI but not
/// auto-imported (re-hashing a whole card on every restart would be wasteful).
pub fn spawn_watcher(app: Shared) {
    tokio::spawn(async move {
        let mut known: HashSet<PathBuf> = devices::list_mounts()
            .into_iter()
            .map(|m| m.mount_point)
            .collect();
        loop {
            tokio::time::sleep(POLL).await;
            let mounts = tokio::task::spawn_blocking(devices::list_mounts)
                .await
                .unwrap_or_default();
            let current: HashSet<PathBuf> = mounts.iter().map(|m| m.mount_point.clone()).collect();
            let cfg = app.config();
            let backups = backup_uuids(&app);
            for m in mounts.iter().filter(|m| !known.contains(&m.mount_point)) {
                let uuid = m.uuid.clone().unwrap_or_default().to_ascii_lowercase();
                let name = m
                    .label
                    .clone()
                    .unwrap_or_else(|| m.mount_point.display().to_string());
                if backups.contains(&uuid) {
                    let _ = app.events.send(ServerEvent::Device {
                        message: format!("Backup drive {name} connected"),
                    });
                    notify("Backup drive connected", &format!("Backing up to {name}…"));
                    spawn_backup(app.clone(), None);
                    continue;
                }
                let ignored = cfg
                    .import
                    .ignore_fs_uuids
                    .iter()
                    .any(|u| u.eq_ignore_ascii_case(&uuid));
                if !m.is_camera_card || ignored {
                    continue;
                }
                let _ = app.events.send(ServerEvent::Device {
                    message: format!("Camera card {name} detected"),
                });
                if cfg.import.auto_import && !app.importer.is_busy() {
                    tracing::info!(mount = %m.mount_point.display(), "camera card inserted; importing");
                    notify(
                        "Camera card detected",
                        &format!("Importing photos from {name}…"),
                    );
                    crate::spawn_import(app.clone(), m.mount_point.clone(), m.uuid.clone());
                }
            }
            known = current;
        }
    });
}

/// After each successful import: desktop notification and (optionally) a backup.
pub fn spawn_import_followups(app: Shared) {
    let mut rx = app.importer.subscribe();
    tokio::spawn(async move {
        while let Ok(ev) = rx.recv().await {
            match ev {
                ImportEvent::Finished { batch } => {
                    notify(
                        "Import complete",
                        &format!(
                            "{} new photos ({} with birds), {} already in library",
                            batch.photos_imported, batch.bird_count, batch.skipped_duplicates
                        ),
                    );
                    if batch.photos_imported > 0 && app.config().backup.after_import {
                        spawn_backup(app.clone(), None);
                    }
                }
                ImportEvent::Failed { error, .. } => notify("Import failed", &error),
                _ => {}
            }
        }
    });
}

pub fn spawn_schedule(app: Shared) {
    tokio::spawn(async move {
        loop {
            let at = app.config().backup.daily_at.clone();
            let Some(secs) = at.as_deref().and_then(photolake_backup::seconds_until) else {
                tokio::time::sleep(Duration::from_secs(600)).await; // re-check config later
                continue;
            };
            // Sleep in bounded steps so a config change takes effect.
            tokio::time::sleep(Duration::from_secs(secs.min(600))).await;
            if secs <= 600 {
                // Nightly maintenance: merge small files, then back up the result.
                match app.lake.compact_all(50).await {
                    Ok(done) => done.iter().for_each(|(t, b, a)| tracing::info!(table = %t, before = b, after = a, "nightly compaction")),
                    Err(e) => tracing::warn!(error = %format!("{e:#}"), "nightly compaction failed"),
                }
                tracing::info!("scheduled backup");
                spawn_backup(app.clone(), None);
                tokio::time::sleep(Duration::from_secs(61)).await;
            }
        }
    });
}

/// Run a backup (one target by name, or all available) in the background.
pub fn spawn_backup(app: Shared, target: Option<String>) {
    tokio::spawn(async move {
        if app.backup.is_running() {
            return;
        }
        let cfg = app.config();
        let events = app.events.clone();
        let progress = move |p: Progress| {
            let _ = events.send(ServerEvent::Backup {
                status: serde_json::json!({ "running": true, "progress": p }),
            });
        };
        let results = match target {
            Some(name) => match cfg.backup.targets.iter().find(|t| t.name == name) {
                Some(t) => vec![app.backup.run(&cfg, t, progress).await],
                None => vec![],
            },
            None => app.backup.run_all(&cfg, progress).await,
        };
        for r in &results {
            if r.ok {
                notify(
                    "Backup complete",
                    &format!(
                        "{}: {} new files ({:.1} GB)",
                        r.target,
                        r.files_copied,
                        r.bytes_copied as f64 / 1e9
                    ),
                );
            } else {
                notify(
                    "Backup failed",
                    &format!("{}: {}", r.target, r.error.clone().unwrap_or_default()),
                );
            }
        }
        let _ = app.events.send(ServerEvent::Backup {
            status: serde_json::json!({ "running": false, "results": results }),
        });
    });
}
