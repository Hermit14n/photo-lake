//! Backups of the whole lakehouse: both buckets (originals + Iceberg warehouse) and a
//! consistent snapshot of the SQLite catalog.
//!
//! Layout at a target:
//!   <root>/photos/...            mirror of the photos bucket
//!   <root>/warehouse/...         mirror of the Iceberg warehouse bucket
//!   <root>/catalog/catalog.db    latest catalog snapshot (+ dated copies)
//!   <root>/BACKUP.json           when/what was backed up
//!
//! Backups are additive: objects are never deleted at the target, so an accidental deletion in
//! the library can't propagate. Iceberg data files are immutable, so runs are incremental.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Local, Utc};
use futures::{StreamExt, TryStreamExt};
use photolake_core::config::{BackupTarget, BackupTargetKind, StorageConfig};
use photolake_core::{Config, devices, store::ObjectStore};
use serde::{Deserialize, Serialize};
use sqlx::Connection;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunResult {
    pub target: String,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub ok: bool,
    pub error: Option<String>,
    pub files_copied: u64,
    pub bytes_copied: u64,
    pub files_total: u64,
    pub location: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub target: String,
    pub phase: String,
    pub done: u64,
    pub total: u64,
}

/// Resolved destination.
enum Dest {
    Dir(PathBuf),
    Rclone(String),
}

fn resolve(target: &BackupTarget) -> Result<Dest> {
    match &target.kind {
        BackupTargetKind::Local {
            fs_uuid: Some(uuid),
            path,
        } => {
            validate_target(target, &[])?;
            let mp = devices::mount_point_for_uuid(uuid)
                .ok_or_else(|| anyhow!("drive {uuid} is not plugged in"))?;
            Ok(Dest::Dir(mp.join(path)))
        }
        BackupTargetKind::Local {
            fs_uuid: None,
            path,
        } => {
            if !path.is_absolute() {
                bail!("local backup path must be absolute: {}", path.display());
            }
            Ok(Dest::Dir(path.clone()))
        }
        BackupTargetKind::Rclone { remote } => {
            validate_target(target, &[])?;
            Ok(Dest::Rclone(remote.trim_end_matches('/').to_string()))
        }
    }
}

/// Reject target definitions that could write outside a backup location or smuggle flags
/// into rclone. Local folders must be on a drive (relative to it) or under `roots`/$HOME.
pub fn validate_target(target: &BackupTarget, roots: &[PathBuf]) -> Result<()> {
    if !target
        .name
        .chars()
        .all(|c| c.is_alphanumeric() || " _-.".contains(c))
        || target.name.len() > 60
    {
        bail!("names may use letters, digits, spaces, '-', '_' and '.'");
    }
    let no_parent = |p: &Path| {
        !p.components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    };
    match &target.kind {
        BackupTargetKind::Local {
            fs_uuid: Some(uuid),
            path,
        } => {
            if uuid.trim().is_empty()
                || !uuid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            {
                bail!("invalid drive id {uuid:?}");
            }
            if path.is_absolute() || !no_parent(path) || path.as_os_str().is_empty() {
                bail!("folder must be a path inside the drive, like \"photolake-backup\"");
            }
        }
        BackupTargetKind::Local {
            fs_uuid: None,
            path,
        } => {
            if !path.is_absolute() || !no_parent(path) {
                bail!("folder must be an absolute path without '..'");
            }
            let home = std::env::var_os("HOME").map(PathBuf::from);
            let ok = roots
                .iter()
                .chain(home.as_ref())
                .any(|r| path.starts_with(r) && path != r);
            if !ok {
                bail!("folder must be inside your home folder or on a removable drive");
            }
        }
        BackupTargetKind::Rclone { remote } => {
            let (name, _) = remote
                .split_once(':')
                .ok_or_else(|| anyhow!("rclone remote must look like \"name:path\""))?;
            if name.is_empty()
                || name.starts_with('-')
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-. ".contains(c))
                || remote.chars().any(|c| c.is_control())
                || remote.starts_with("photolakesrc:")
            {
                bail!("invalid rclone remote {remote:?}");
            }
        }
    }
    Ok(())
}

/// Whether the target can run right now (e.g. its drive is plugged in).
pub fn available(target: &BackupTarget) -> Result<String> {
    match resolve(target)? {
        Dest::Dir(p) => Ok(p.display().to_string()),
        Dest::Rclone(r) => {
            rclone_version()
                .ok_or_else(|| anyhow!("rclone is not installed (sudo apt install rclone)"))?;
            Ok(r)
        }
    }
}

pub fn rclone_version() -> Option<String> {
    let out = std::process::Command::new("rclone")
        .arg("version")
        .output()
        .ok()?;
    out.status.success().then(|| {
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .to_string()
    })
}

/// Consistent copy of the live SQLite catalog (safe while the server is writing).
async fn snapshot_catalog(cfg: &Config, out: &Path) -> Result<()> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(out);
    let mut conn =
        sqlx::SqliteConnection::connect(&format!("sqlite://{}", cfg.catalog_db_path().display()))
            .await?;
    let path = out.display().to_string().replace('\'', "''");
    sqlx::query(&format!("VACUUM INTO '{path}'"))
        .execute(&mut conn)
        .await
        .context("snapshotting catalog")?;
    Ok(())
}

pub struct BackupManager {
    state_path: PathBuf,
    state: Mutex<HashMap<String, RunResult>>,
    running: tokio::sync::Mutex<()>,
}

impl BackupManager {
    pub fn new(cfg: &Config) -> Self {
        let state_path = cfg.data_dir().join("backup_state.json");
        let state = std::fs::read_to_string(&state_path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Self {
            state_path,
            state: Mutex::new(state),
            running: tokio::sync::Mutex::new(()),
        }
    }

    pub fn last_results(&self) -> HashMap<String, RunResult> {
        self.state.lock().unwrap().clone()
    }

    pub fn is_running(&self) -> bool {
        self.running.try_lock().is_err()
    }

    fn record(&self, r: RunResult) {
        let mut st = self.state.lock().unwrap();
        st.insert(r.target.clone(), r);
        if let Ok(text) = serde_json::to_string_pretty(&*st) {
            let _ = std::fs::write(&self.state_path, text);
        }
    }

    /// Back up to every enabled target that is currently reachable.
    pub async fn run_all(
        &self,
        cfg: &Config,
        progress: impl Fn(Progress) + Clone,
    ) -> Vec<RunResult> {
        let mut out = Vec::new();
        for t in cfg.backup.targets.iter().filter(|t| t.enabled) {
            if let Err(e) = available(t) {
                tracing::info!(target = %t.name, reason = %e, "skipping backup target");
                continue;
            }
            out.push(self.run(cfg, t, progress.clone()).await);
        }
        out
    }

    pub async fn run(
        &self,
        cfg: &Config,
        target: &BackupTarget,
        progress: impl Fn(Progress),
    ) -> RunResult {
        let _guard = self.running.lock().await;
        let mut r = RunResult {
            target: target.name.clone(),
            started_at: Some(Utc::now()),
            ..Default::default()
        };
        let res = match resolve(target) {
            Ok(Dest::Dir(dir)) => {
                r.location = Some(dir.display().to_string());
                backup_to_dir(cfg, &dir, &target.name, &mut r, &progress).await
            }
            Ok(Dest::Rclone(remote)) => {
                r.location = Some(remote.clone());
                backup_rclone(cfg, &remote, &target.name, &progress).await
            }
            Err(e) => Err(e),
        };
        r.finished_at = Some(Utc::now());
        match res {
            Ok(()) => r.ok = true,
            Err(e) => {
                tracing::error!(target = %target.name, error = %format!("{e:#}"), "backup failed");
                r.error = Some(format!("{e:#}"));
            }
        }
        tracing::info!(target = %r.target, ok = r.ok, files = r.files_copied, bytes = r.bytes_copied, "backup finished");
        self.record(r.clone());
        r
    }
}

async fn backup_to_dir(
    cfg: &Config,
    root: &Path,
    name: &str,
    r: &mut RunResult,
    progress: &impl Fn(Progress),
) -> Result<()> {
    std::fs::create_dir_all(root).with_context(|| format!("creating {}", root.display()))?;
    let s = &cfg.storage;
    for (bucket, sub) in [
        (&s.warehouse_bucket, "warehouse"),
        (&s.photos_bucket, "photos"),
    ] {
        let store = ObjectStore::new(s, bucket)?;
        let (copied, bytes, total) =
            mirror_bucket(&store, &root.join(sub), name, sub, progress).await?;
        r.files_copied += copied;
        r.bytes_copied += bytes;
        r.files_total += total;
    }
    // Catalog last: every file it references is already at the target.
    progress(Progress {
        target: name.into(),
        phase: "catalog".into(),
        done: 0,
        total: 1,
    });
    let cat_dir = root.join("catalog");
    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    let dated = cat_dir.join(format!("catalog-{stamp}.db"));
    snapshot_catalog(cfg, &dated).await?;
    let latest_tmp = cat_dir.join("catalog.db.tmp");
    std::fs::copy(&dated, &latest_tmp)?;
    std::fs::rename(&latest_tmp, cat_dir.join("catalog.db"))?;
    prune_dated(&cat_dir, 30)?;
    std::fs::write(
        root.join("BACKUP.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "tool": "photolake",
            "finished_at": Utc::now(),
            "photos_bucket": s.photos_bucket,
            "warehouse_bucket": s.warehouse_bucket,
            "objects": r.files_total,
            "restore": "photolake restore --from <this folder>",
        }))?,
    )?;
    Ok(())
}

/// Keep the newest `keep` dated catalog snapshots.
fn prune_dated(dir: &Path, keep: usize) -> Result<()> {
    let mut dated: Vec<PathBuf> = std::fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("catalog-"))
        })
        .collect();
    dated.sort();
    if dated.len() > keep {
        for p in &dated[..dated.len() - keep] {
            let _ = std::fs::remove_file(p);
        }
    }
    Ok(())
}

/// Copy objects missing (or different in size) at `dir`. Returns (copied, bytes, total).
async fn mirror_bucket(
    store: &ObjectStore,
    dir: &Path,
    name: &str,
    phase: &str,
    progress: &impl Fn(Progress),
) -> Result<(u64, u64, u64)> {
    let entries: Vec<_> = store
        .operator()
        .lister_with("")
        .recursive(true)
        .await?
        .try_filter(|e| futures::future::ready(e.metadata().is_file()))
        .try_collect()
        .await?;
    let total = entries.len() as u64;
    let mut todo = Vec::new();
    for e in &entries {
        let dest = dir.join(e.path());
        let size = e.metadata().content_length();
        let have = std::fs::metadata(&dest).map(|m| m.len()).ok();
        // Listing may omit sizes on some servers; fall back to a stat.
        let size = if size == 0 {
            store.size(e.path()).await?.unwrap_or(0)
        } else {
            size
        };
        if have != Some(size) {
            todo.push((e.path().to_string(), dest));
        }
    }
    let n = todo.len() as u64;
    progress(Progress {
        target: name.into(),
        phase: phase.into(),
        done: 0,
        total: n,
    });
    let mut done = 0u64;
    let mut bytes = 0u64;
    let jobs: Vec<_> = todo
        .into_iter()
        .map(|(key, dest)| {
            let store = store.clone();
            async move {
                let data = store.get(&key).await?;
                if let Some(parent) = dest.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                let tmp = dest.with_extension("partial");
                tokio::fs::write(&tmp, &data).await?;
                tokio::fs::rename(&tmp, &dest).await?;
                Ok::<u64, anyhow::Error>(data.len() as u64)
            }
        })
        .collect();
    let mut stream = futures::stream::iter(jobs).buffer_unordered(8);
    while let Some(res) = stream.next().await {
        bytes += res?;
        done += 1;
        if done.is_multiple_of(50) || done == n {
            progress(Progress {
                target: name.into(),
                phase: phase.into(),
                done,
                total: n,
            });
        }
    }
    Ok((n, bytes, total))
}

/// rclone remote definition for the local object store, passed via environment variables.
fn rclone_source_env(s: &StorageConfig) -> Vec<(String, String)> {
    [
        ("TYPE", "s3"),
        ("PROVIDER", "Other"),
        ("ENDPOINT", s.endpoint.as_str()),
        ("ACCESS_KEY_ID", s.access_key.as_str()),
        ("SECRET_ACCESS_KEY", s.secret_key.as_str()),
        ("REGION", s.region.as_str()),
        ("FORCE_PATH_STYLE", "true"),
    ]
    .into_iter()
    .map(|(k, v)| (format!("RCLONE_CONFIG_PHOTOLAKESRC_{k}"), v.to_string()))
    .collect()
}

async fn rclone(args: &[String], env: &[(String, String)]) -> Result<()> {
    let out = tokio::process::Command::new("rclone")
        .args(args)
        .envs(env.iter().cloned())
        .output()
        .await
        .context("running rclone (is it installed?)")?;
    if !out.status.success() {
        bail!(
            "rclone {}: {}",
            args.first().map(String::as_str).unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

async fn backup_rclone(
    cfg: &Config,
    remote: &str,
    name: &str,
    progress: &impl Fn(Progress),
) -> Result<()> {
    let env = rclone_source_env(&cfg.storage);
    let s = &cfg.storage;
    for (bucket, sub) in [
        (&s.warehouse_bucket, "warehouse"),
        (&s.photos_bucket, "photos"),
    ] {
        progress(Progress {
            target: name.into(),
            phase: sub.into(),
            done: 0,
            total: 1,
        });
        // `copy` (not `sync`): never delete at the destination.
        rclone(
            &[
                "copy".into(),
                format!("photolakesrc:{bucket}"),
                format!("{remote}/{sub}"),
                "--transfers=8".into(),
            ],
            &env,
        )
        .await?;
    }
    progress(Progress {
        target: name.into(),
        phase: "catalog".into(),
        done: 0,
        total: 1,
    });
    let tmp = cfg.data_dir().join("catalog-backup.db");
    snapshot_catalog(cfg, &tmp).await?;
    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    for dst in [
        format!("{remote}/catalog/catalog-{stamp}.db"),
        format!("{remote}/catalog/catalog.db"),
    ] {
        rclone(&["copyto".into(), tmp.display().to_string(), dst], &env).await?;
    }
    let _ = std::fs::remove_file(tmp);
    Ok(())
}

/// Restore a backup folder into the object store and catalog location of `cfg`.
pub async fn restore_from_dir(
    cfg: &Config,
    root: &Path,
    force: bool,
    progress: impl Fn(&str, u64, u64),
) -> Result<()> {
    let catalog = root.join("catalog/catalog.db");
    if !catalog.exists() {
        bail!(
            "{} is not a photolake backup (no catalog/catalog.db)",
            root.display()
        );
    }
    let dest_db = cfg.catalog_db_path();
    if dest_db.exists() && !force {
        bail!(
            "a catalog already exists at {}; pass --force to replace it",
            dest_db.display()
        );
    }
    photolake_core::store::ensure_buckets(&cfg.storage).await?;
    let s = &cfg.storage;
    for (bucket, sub) in [
        (&s.photos_bucket, "photos"),
        (&s.warehouse_bucket, "warehouse"),
    ] {
        let store = ObjectStore::new(s, bucket)?;
        let base = root.join(sub);
        let files: Vec<PathBuf> = walk(&base);
        let total = files.len() as u64;
        let mut done = 0;
        for f in files {
            let key = f.strip_prefix(&base)?.to_string_lossy().replace('\\', "/");
            let len = std::fs::metadata(&f)?.len();
            if store.size(&key).await? != Some(len) {
                let data = tokio::fs::read(&f).await?;
                let ct = if key.ends_with(".jpg") {
                    "image/jpeg"
                } else {
                    "application/octet-stream"
                };
                store.put(&key, data, ct).await?;
            }
            done += 1;
            if done % 100 == 0 || done == total {
                progress(sub, done, total);
            }
        }
    }
    if let Some(dir) = dest_db.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::copy(&catalog, &dest_db)?;
    Ok(())
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_none_or(|x| x != "partial") {
                out.push(p);
            }
        }
    }
    out
}

/// Seconds until the next local "HH:MM".
pub fn seconds_until(hhmm: &str) -> Option<u64> {
    let t = chrono::NaiveTime::parse_from_str(hhmm.trim(), "%H:%M").ok()?;
    let now = Local::now();
    let today = now.date_naive().and_time(t);
    let next = if today > now.naive_local() {
        today
    } else {
        today + chrono::Duration::days(1)
    };
    Some((next - now.naive_local()).num_seconds().max(1) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_validate() {
        let t = |kind| BackupTarget {
            name: "t".into(),
            enabled: true,
            kind,
        };
        let roots = vec![PathBuf::from("/media/me")];
        let rc = |r: &str| BackupTargetKind::Rclone { remote: r.into() };
        assert!(validate_target(&t(rc("b2:bucket/photolake")), &roots).is_ok());
        assert!(validate_target(&t(rc("--config=/x:y")), &roots).is_err());
        assert!(validate_target(&t(rc("nocolon")), &roots).is_err());
        let drive = |p: &str| BackupTargetKind::Local {
            fs_uuid: Some("ABCD-1234".into()),
            path: p.into(),
        };
        assert!(validate_target(&t(drive("photolake-backup")), &roots).is_ok());
        assert!(validate_target(&t(drive("../../etc")), &roots).is_err());
        assert!(validate_target(&t(drive("/etc")), &roots).is_err());
        let dir = |p: &str| BackupTargetKind::Local {
            fs_uuid: None,
            path: p.into(),
        };
        assert!(validate_target(&t(dir("/media/me/disk/bk")), &roots).is_ok());
        assert!(validate_target(&t(dir("/etc/cron.d")), &roots).is_err());
    }

    #[test]
    fn schedule_parses() {
        let s = super::seconds_until("02:30").unwrap();
        assert!(s > 0 && s <= 86_400);
        assert!(super::seconds_until("25:00").is_none());
    }
}
