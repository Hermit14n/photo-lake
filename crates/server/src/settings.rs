//! Settings (persisted to config.toml), backup status/control, and drive listing.

use axum::Json;
use axum::extract::State;
use photolake_core::config::BackupTarget;
use photolake_core::devices;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{ApiResult, bad_request};
use crate::state::Shared;

pub async fn get_settings(State(app): State<Shared>) -> Json<Value> {
    let c = app.config();
    Json(json!({
        "config_path": app.cfg_path,
        "import": {
            "auto_import": c.import.auto_import,
            "camera_timezone": c.import.camera_timezone,
            "ignore_fs_uuids": c.import.ignore_fs_uuids,
        },
        "ml": {
            "enabled": c.ml.enabled,
            "detection_threshold": c.ml.detection_threshold,
            "has_ebird_key": c.ml.ebird_api_key.as_deref().is_some_and(|k| !k.is_empty()),
            "home_region": c.ml.home_region,
            "tiling": c.ml.tiling,
            "device": c.ml.device,
        },
        "backup": {
            "after_import": c.backup.after_import,
            "daily_at": c.backup.daily_at,
            "targets": c.backup.targets,
        },
    }))
}

#[derive(Debug, Deserialize)]
pub struct SettingsPatch {
    pub auto_import: Option<bool>,
    /// Empty string clears.
    pub camera_timezone: Option<String>,
    pub ml_enabled: Option<bool>,
    pub detection_threshold: Option<f32>,
    pub ebird_api_key: Option<String>,
    pub home_region: Option<String>,
    pub tiling: Option<bool>,
    pub after_import: Option<bool>,
    pub daily_at: Option<String>,
    pub targets: Option<Vec<BackupTarget>>,
}

pub async fn put_settings(
    State(app): State<Shared>,
    Json(p): Json<SettingsPatch>,
) -> ApiResult<Json<Value>> {
    let path = app
        .cfg_path
        .clone()
        .ok_or_else(|| bad_request("no config file location; start with --config"))?;
    let mut restart = false;
    {
        let mut c = app.cfg.write().unwrap();
        let opt = |s: String| (!s.trim().is_empty()).then(|| s.trim().to_string());
        if let Some(v) = p.auto_import {
            c.import.auto_import = v;
        }
        if let Some(v) = p.camera_timezone {
            if let Some(z) = opt(v.clone()) {
                z.parse::<chrono_tz::Tz>()
                    .map_err(|_| bad_request(format!("unknown time zone {z}")))?;
            }
            c.import.camera_timezone = opt(v);
            restart = true;
        }
        if let Some(v) = p.ml_enabled {
            c.ml.enabled = v;
            restart = true;
        }
        if let Some(v) = p.detection_threshold {
            if !(0.05..=0.95).contains(&v) {
                return Err(bad_request("threshold must be between 0.05 and 0.95"));
            }
            c.ml.detection_threshold = v;
            restart = true;
        }
        if let Some(v) = p.ebird_api_key {
            c.ml.ebird_api_key = opt(v);
            restart = true;
        }
        if let Some(v) = p.home_region {
            c.ml.home_region = opt(v.to_uppercase());
            restart = true;
        }
        if let Some(v) = p.tiling {
            c.ml.tiling = v;
            restart = true;
        }
        if let Some(v) = p.after_import {
            c.backup.after_import = v;
        }
        if let Some(v) = p.daily_at {
            match opt(v) {
                Some(t) if photolake_backup::seconds_until(&t).is_none() => {
                    return Err(bad_request("daily time must be HH:MM (24h)"));
                }
                t => c.backup.daily_at = t,
            }
        }
        if let Some(targets) = p.targets {
            let mut names = std::collections::HashSet::new();
            for t in &targets {
                if t.name.trim().is_empty() || !names.insert(t.name.clone()) {
                    return Err(bad_request("each backup target needs a unique name"));
                }
                photolake_backup::validate_target(t, &c.import.mount_roots)
                    .map_err(|e| bad_request(format!("{}: {e}", t.name)))?;
            }
            c.backup.targets = targets;
        }
        c.save(&path)?;
    }
    Ok(Json(json!({ "saved": path, "restart_required": restart })))
}

pub async fn backup_status(State(app): State<Shared>) -> Json<Value> {
    let cfg = app.config();
    let last = app.backup.last_results();
    let targets: Vec<Value> = cfg
        .backup
        .targets
        .iter()
        .map(|t| {
            let avail = photolake_backup::available(t);
            json!({
                "target": t,
                "available": avail.is_ok(),
                "location": avail.as_ref().ok(),
                "reason": avail.as_ref().err().map(|e| e.to_string()),
                "last": last.get(&t.name),
            })
        })
        .collect();
    Json(json!({
        "running": app.backup.is_running(),
        "rclone": photolake_backup::rclone_version(),
        "daily_at": cfg.backup.daily_at,
        "after_import": cfg.backup.after_import,
        "targets": targets,
    }))
}

#[derive(Deserialize)]
pub struct RunBody {
    pub target: Option<String>,
}

pub async fn backup_run(
    State(app): State<Shared>,
    Json(b): Json<RunBody>,
) -> ApiResult<Json<Value>> {
    if app.backup.is_running() {
        return Err(bad_request("a backup is already running"));
    }
    if app.config().backup.targets.is_empty() {
        return Err(bad_request("add a backup target first"));
    }
    crate::daemon::spawn_backup(app.clone(), b.target);
    Ok(Json(json!({ "started": true })))
}

pub async fn drives() -> Json<Value> {
    Json(json!(
        tokio::task::spawn_blocking(devices::list_mounts)
            .await
            .unwrap_or_default()
    ))
}
