use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use anyhow::Result;
use photolake_core::{Config, store::ObjectStore};
use photolake_ingest::Importer;
use photolake_lake::Lake;
use photolake_ml::manifest::Species;
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::events::ServerEvent;

/// Object keys for one photo; these never change after import.
#[derive(Debug, Clone, Deserialize)]
pub struct Keys {
    pub thumb_key: String,
    pub preview_key: String,
    pub jpeg_key: Option<String>,
    pub raw_key: Option<String>,
}

pub struct AppState {
    pub cfg: RwLock<Config>,
    pub cfg_path: Option<PathBuf>,
    pub lake: Arc<Lake>,
    pub store: ObjectStore,
    pub importer: Arc<Importer>,
    pub keys: RwLock<HashMap<String, Keys>>,
    /// eBird taxonomy for species pickers (empty when models aren't installed).
    pub species: Vec<Species>,
    pub species_by_code: HashMap<String, usize>,
    pub events: broadcast::Sender<ServerEvent>,
    pub backup: Arc<photolake_backup::BackupManager>,
    pub auth: crate::auth::Auth,
    /// A "re-run bird ML" job is in progress.
    pub reprocessing: std::sync::atomic::AtomicBool,
}

pub type Shared = Arc<AppState>;

impl AppState {
    pub fn config(&self) -> Config {
        self.cfg.read().unwrap().clone()
    }

    /// Reload the photo → object-key index (after imports).
    pub async fn reload_keys(&self) -> Result<()> {
        #[derive(Deserialize)]
        struct Row {
            photo_id: String,
            #[serde(flatten)]
            keys: Keys,
        }
        let rows = self
            .lake
            .query_json(&format!(
                "SELECT photo_id, thumb_key, preview_key, jpeg_key, raw_key FROM {}",
                Lake::table_name(photolake_lake::PHOTOS)
            ))
            .await?;
        let mut map = HashMap::with_capacity(rows.len());
        for r in rows {
            let r: Row = serde_json::from_value(r)?;
            map.insert(r.photo_id, r.keys);
        }
        *self.keys.write().unwrap() = map;
        Ok(())
    }

    pub fn species_lookup(&self, code: &str) -> Option<&Species> {
        self.species_by_code.get(code).map(|&i| &self.species[i])
    }
}
