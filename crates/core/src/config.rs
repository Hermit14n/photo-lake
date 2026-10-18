//! Application configuration, loaded from TOML.
//!
//! Lookup order: `--config <path>`, `$PHOTOLAKE_CONFIG`, `~/.config/photolake/config.toml`.
//! Every field has a default, so an empty file (or no file) yields a working local setup.
//!
//! Selected settings can be overridden by environment variables (see [`ENV_OVERRIDES`]); this
//! lets one config file work both natively and inside a container. Overrides are never
//! written back to the file.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    /// Where local state lives (catalog db, caches, logs).
    pub data_dir: Option<PathBuf>,
    pub storage: StorageConfig,
    pub server: ServerConfig,
    pub import: ImportConfig,
    pub ml: MlConfig,
    pub backup: BackupConfig,
    /// The config as read from the file, before environment overrides (used by `save`).
    #[serde(skip)]
    file: Option<Box<Config>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    pub endpoint: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    /// Bucket for original files, previews and thumbnails.
    pub photos_bucket: String,
    /// Bucket holding the Iceberg warehouse (Parquet data + table metadata).
    pub warehouse_bucket: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:9000".into(),
            region: "us-east-1".into(),
            access_key: "photolake".into(),
            secret_key: "photolake-secret".into(),
            photos_bucket: "photos".into(),
            warehouse_bucket: "warehouse".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub bind: String,
    /// Directory containing the built web UI (`web/dist`).
    pub web_dir: Option<PathBuf>,
    /// Extra host names the UI may be reached under (e.g. "photolake.tail1234.ts.net").
    /// Loopback names and this machine's own addresses are always allowed.
    pub allowed_hosts: Vec<String>,
    pub auth: AuthConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:7878".into(),
            web_dir: None,
            allowed_hosts: vec![],
            auth: AuthConfig::default(),
        }
    }
}

/// Login. Without an admin password the UI is open, which is only allowed while the
/// server listens on loopback.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthConfig {
    /// Argon2 hash; set with `photolake passwd`.
    pub admin_password_hash: Option<String>,
    /// Optional read-only login; set with `photolake passwd --viewer`.
    pub viewer_password_hash: Option<String>,
    pub session_days: u32,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            admin_password_hash: None,
            viewer_password_hash: None,
            session_days: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportConfig {
    /// Watch for newly mounted cards and import automatically.
    pub auto_import: bool,
    /// Directories under which removable media get mounted.
    pub mount_roots: Vec<PathBuf>,
    /// Filesystem UUIDs that are never imported from (e.g. backup drives).
    pub ignore_fs_uuids: Vec<String>,
    /// Long edge of the stored preview JPEG.
    pub preview_px: u32,
    /// Long edge of the stored thumbnail.
    pub thumb_px: u32,
    /// Parallel uploads to object storage.
    pub upload_concurrency: usize,
    /// Optional time zone for cameras that don't record an offset, e.g. "America/New_York".
    /// Without it, naive EXIF times are treated as local system time.
    pub camera_timezone: Option<String>,
}

impl Default for ImportConfig {
    fn default() -> Self {
        let user = std::env::var("USER").unwrap_or_default();
        Self {
            auto_import: true,
            mount_roots: vec![
                PathBuf::from(format!("/media/{user}")),
                PathBuf::from(format!("/run/media/{user}")),
            ],
            ignore_fs_uuids: vec![],
            preview_px: 2048,
            thumb_px: 480,
            upload_concurrency: 8,
            camera_timezone: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MlConfig {
    pub enabled: bool,
    pub models_dir: Option<PathBuf>,
    /// Minimum detector confidence for a "bird" box.
    pub detection_threshold: f32,
    /// Number of species candidates kept per bird.
    pub top_k: usize,
    /// "auto" (GPU when available, else CPU), "cuda" (GPU or fail) or "cpu".
    pub device: String,
    /// eBird API key for location/season re-ranking (optional, free at ebird.org/api/keygen).
    pub ebird_api_key: Option<String>,
    /// eBird region used when a photo has no GPS, e.g. "US-PA" or "GB-ENG".
    pub home_region: Option<String>,
    /// Also scan a 2x2 tile grid when the full frame finds no bird (catches distant birds,
    /// costs ~5x detector time on bird-less photos).
    pub tiling: bool,
}

impl Default for MlConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            models_dir: None,
            detection_threshold: 0.4,
            top_k: 3,
            device: "auto".into(),
            ebird_api_key: None,
            home_region: None,
            tiling: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupConfig {
    /// Run a backup after every successful import.
    pub after_import: bool,
    /// Daily backup time, local "HH:MM". None disables the schedule.
    pub daily_at: Option<String>,
    pub targets: Vec<BackupTarget>,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            after_import: true,
            daily_at: Some("02:30".into()),
            targets: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BackupTargetKind {
    /// A local directory, usually on an external drive. When `fs_uuid` is set the
    /// drive's mount point is discovered automatically.
    Local {
        #[serde(default)]
        fs_uuid: Option<String>,
        /// Absolute path, or relative to the drive's mount point when `fs_uuid` is set.
        path: PathBuf,
    },
    /// Any rclone remote ("b2:bucket/path", "gdrive:Photolake", "s3:bucket", ...).
    Rclone { remote: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackupTarget {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(flatten)]
    pub kind: BackupTargetKind,
}

fn default_true() -> bool {
    true
}

/// Environment variables that override config values.
pub const ENV_OVERRIDES: &[&str] = &[
    "PHOTOLAKE_DATA_DIR",
    "PHOTOLAKE_MODELS_DIR",
    "PHOTOLAKE_WEB_DIR",
    "PHOTOLAKE_BIND",
    "PHOTOLAKE_ALLOWED_HOSTS",
    "PHOTOLAKE_S3_ENDPOINT",
    "PHOTOLAKE_S3_ACCESS_KEY",
    "PHOTOLAKE_S3_SECRET_KEY",
    "PHOTOLAKE_MOUNT_ROOTS",
    "PHOTOLAKE_ML_DEVICE",
];

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn list(v: &str, sep: char) -> Vec<String> {
    v.split(sep)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

impl Config {
    /// Apply [`ENV_OVERRIDES`], remembering the file values so `save` doesn't persist them.
    pub fn apply_env(&mut self) {
        let mut file = self.clone();
        file.file = None;
        if let Some(v) = env("PHOTOLAKE_DATA_DIR") {
            self.data_dir = Some(v.into());
        }
        if let Some(v) = env("PHOTOLAKE_MODELS_DIR") {
            self.ml.models_dir = Some(v.into());
        }
        if let Some(v) = env("PHOTOLAKE_WEB_DIR") {
            self.server.web_dir = Some(v.into());
        }
        if let Some(v) = env("PHOTOLAKE_BIND") {
            self.server.bind = v;
        }
        if let Some(v) = env("PHOTOLAKE_ALLOWED_HOSTS") {
            self.server.allowed_hosts.extend(list(&v, ','));
        }
        if let Some(v) = env("PHOTOLAKE_S3_ENDPOINT") {
            self.storage.endpoint = v;
        }
        if let Some(v) = env("PHOTOLAKE_S3_ACCESS_KEY") {
            self.storage.access_key = v;
        }
        if let Some(v) = env("PHOTOLAKE_S3_SECRET_KEY") {
            self.storage.secret_key = v;
        }
        if let Some(v) = env("PHOTOLAKE_MOUNT_ROOTS") {
            self.import.mount_roots = list(&v, ':').into_iter().map(PathBuf::from).collect();
        }
        if let Some(v) = env("PHOTOLAKE_ML_DEVICE") {
            self.ml.device = v;
        }
        self.file = Some(Box::new(file));
    }

    /// This config with environment-overridden values replaced by their file values.
    fn for_file(&self) -> Config {
        let mut out = self.clone();
        out.file = None;
        let Some(f) = &self.file else { return out };
        let set = |name: &str| env(name).is_some();
        if set("PHOTOLAKE_DATA_DIR") {
            out.data_dir = f.data_dir.clone();
        }
        if set("PHOTOLAKE_MODELS_DIR") {
            out.ml.models_dir = f.ml.models_dir.clone();
        }
        if set("PHOTOLAKE_WEB_DIR") {
            out.server.web_dir = f.server.web_dir.clone();
        }
        if set("PHOTOLAKE_BIND") {
            out.server.bind = f.server.bind.clone();
        }
        if set("PHOTOLAKE_ALLOWED_HOSTS") {
            out.server.allowed_hosts = f.server.allowed_hosts.clone();
        }
        if set("PHOTOLAKE_S3_ENDPOINT") {
            out.storage.endpoint = f.storage.endpoint.clone();
        }
        if set("PHOTOLAKE_S3_ACCESS_KEY") {
            out.storage.access_key = f.storage.access_key.clone();
        }
        if set("PHOTOLAKE_S3_SECRET_KEY") {
            out.storage.secret_key = f.storage.secret_key.clone();
        }
        if set("PHOTOLAKE_MOUNT_ROOTS") {
            out.import.mount_roots = f.import.mount_roots.clone();
        }
        if set("PHOTOLAKE_ML_DEVICE") {
            out.ml.device = f.ml.device.clone();
        }
        out
    }

    pub fn default_path() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("PHOTOLAKE_CONFIG") {
            return Some(PathBuf::from(p));
        }
        directories::ProjectDirs::from("", "", "photolake")
            .map(|d| d.config_dir().join("config.toml"))
    }

    /// Load from an explicit path, or the default location. A missing default file is not an error.
    pub fn load(path: Option<&Path>) -> Result<(Self, Option<PathBuf>)> {
        let (path, required) = match path {
            Some(p) => (Some(p.to_path_buf()), true),
            None => (Self::default_path(), false),
        };
        let Some(path) = path else {
            let mut cfg = Self::default();
            cfg.apply_env();
            return Ok((cfg, None));
        };
        if !path.exists() && !required {
            let mut cfg = Self::default();
            cfg.apply_env();
            return Ok((cfg, Some(path)));
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading config {}", path.display()))?;
        let mut cfg: Config =
            toml::from_str(&text).with_context(|| format!("parsing config {}", path.display()))?;
        cfg.apply_env();
        Ok((cfg, Some(path)))
    }

    /// Write the config (it holds secrets, so the file is private to the user).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(&self.for_file())?;
        let tmp = path.with_extension("toml.tmp");
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)
                .with_context(|| format!("writing {}", tmp.display()))?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, path)?;
        restrict(path);
        Ok(())
    }

    pub fn data_dir(&self) -> PathBuf {
        self.data_dir.clone().unwrap_or_else(|| {
            directories::ProjectDirs::from("", "", "photolake")
                .map(|d| d.data_dir().to_path_buf())
                .unwrap_or_else(|| PathBuf::from("./data/local"))
        })
    }

    pub fn catalog_db_path(&self) -> PathBuf {
        self.data_dir().join("catalog.db")
    }

    pub fn models_dir(&self) -> PathBuf {
        self.ml
            .models_dir
            .clone()
            .unwrap_or_else(|| self.data_dir().join("models"))
    }
}

/// Make a file or directory private to the current user (best effort).
pub fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mode = if path.is_dir() { 0o700 } else { 0o600 };
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)) {
        tracing::debug!(error = %e, path = %path.display(), "could not restrict permissions");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_uses_defaults() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.storage.photos_bucket, "photos");
        assert_eq!(cfg.server.bind, "127.0.0.1:7878");
        assert!(cfg.backup.targets.is_empty());
    }

    #[test]
    fn parses_backup_targets() {
        let cfg: Config = toml::from_str(
            r#"
            [[backup.targets]]
            name = "external-hdd"
            kind = "local"
            fs_uuid = "ABCD-1234"
            path = "photolake-backup"

            [[backup.targets]]
            name = "b2"
            kind = "rclone"
            remote = "b2:my-photolake"
            enabled = false
            "#,
        )
        .unwrap();
        assert_eq!(cfg.backup.targets.len(), 2);
        assert!(cfg.backup.targets[0].enabled);
        assert_eq!(
            cfg.backup.targets[0].kind,
            BackupTargetKind::Local {
                fs_uuid: Some("ABCD-1234".into()),
                path: "photolake-backup".into()
            }
        );
        assert!(!cfg.backup.targets[1].enabled);
    }
}
