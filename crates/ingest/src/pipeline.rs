//! The import pipeline: scan → hash/dedupe → metadata → render → ML → upload → Iceberg.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate, Utc};
use futures::StreamExt;
use photolake_core::model::{
    BBox, CATEGORY_BIRD, CATEGORY_GENERAL, Detection, ImportBatch, Photo, keys,
};
use photolake_core::{Config, store::ObjectStore};
use photolake_lake::Lake;
use photolake_ml::{AnalyzeContext, Analyzer};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::meta::{self, CameraClock, Meta};
use crate::scan::{self, Shot};
use crate::{geo, render};

/// Photos are committed to Iceberg in chunks so a large card shows up progressively
/// and memory stays bounded.
const COMMIT_CHUNK: usize = 200;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImportEvent {
    Started {
        batch_id: Uuid,
        source: String,
        total: usize,
    },
    Progress {
        batch_id: Uuid,
        done: usize,
        total: usize,
        file: String,
        outcome: String,
    },
    Finished {
        batch: ImportBatch,
    },
    Failed {
        batch_id: Uuid,
        error: String,
    },
}

pub struct ImportOptions {
    pub source_fs_uuid: Option<String>,
}

pub struct Importer {
    cfg: Config,
    lake: Arc<Lake>,
    store: ObjectStore,
    analyzer: Option<Arc<dyn Analyzer>>,
    events: broadcast::Sender<ImportEvent>,
    /// Serializes imports (two cards at once would race on de-duplication).
    lock: tokio::sync::Mutex<()>,
}

/// Everything produced for one shot before upload.
struct Prepared {
    photo: Photo,
    detections: Vec<Detection>,
    uploads: Vec<(String, bytes::Bytes, &'static str)>,
}

enum Outcome {
    Imported(Box<Prepared>),
    Duplicate,
}

impl Importer {
    pub fn new(cfg: Config, lake: Arc<Lake>, analyzer: Option<Arc<dyn Analyzer>>) -> Result<Self> {
        let store = ObjectStore::photos(&cfg.storage)?;
        let (events, _) = broadcast::channel(1024);
        Ok(Self {
            cfg,
            lake,
            store,
            analyzer,
            events,
            lock: tokio::sync::Mutex::new(()),
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ImportEvent> {
        self.events.subscribe()
    }

    /// The bird models in use (None when ML is off or not installed).
    pub fn analyzer(&self) -> Option<Arc<dyn Analyzer>> {
        self.analyzer.clone()
    }

    pub fn is_busy(&self) -> bool {
        self.lock.try_lock().is_err()
    }

    pub async fn import_path(&self, root: &Path, opts: ImportOptions) -> Result<ImportBatch> {
        let _guard = self.lock.lock().await;
        let batch_id = Uuid::now_v7();
        let res = self.run(batch_id, root, opts).await;
        if let Err(e) = &res {
            let _ = self.events.send(ImportEvent::Failed {
                batch_id,
                error: format!("{e:#}"),
            });
        }
        res
    }

    async fn run(&self, batch_id: Uuid, root: &Path, opts: ImportOptions) -> Result<ImportBatch> {
        let started_at = Utc::now();
        let import_date = Local::now().date_naive();
        let root_owned = root.to_path_buf();
        let scan = tokio::task::spawn_blocking(move || scan::scan(&root_owned)).await?;
        let total = scan.shots.len();
        tracing::info!(%batch_id, source = %root.display(), total, "import started");
        let _ = self.events.send(ImportEvent::Started {
            batch_id,
            source: root.display().to_string(),
            total,
        });

        let known = Arc::new(Mutex::new(self.lake.known_hashes().await?));
        let clock = Arc::new(CameraClock::from_config(
            self.cfg.import.camera_timezone.as_deref(),
        ));
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .clamp(2, 8);

        let (mut imported, mut dups, mut errors, mut birds) = (0i64, 0i64, 0i64, 0i64);
        let mut done = 0usize;
        let mut pending: Vec<Prepared> = Vec::new();

        // Futures are built eagerly (collected into a Vec) so the resulting stream is
        // provably `Send` for spawning; they don't start until polled.
        let jobs: Vec<_> = scan
            .shots
            .into_iter()
            .map(|shot| {
                let known = known.clone();
                let clock = clock.clone();
                let analyzer = self.analyzer.clone();
                let cfg = self.cfg.clone();
                async move {
                    let name = shot.display_name();
                    let res = tokio::task::spawn_blocking(move || {
                        prepare(
                            &shot,
                            batch_id,
                            import_date,
                            &cfg,
                            &clock,
                            &known,
                            analyzer.as_deref(),
                        )
                    })
                    .await
                    .map_err(anyhow::Error::from)
                    .and_then(|r| r);
                    (name, res)
                }
            })
            .collect();
        let mut stream = futures::stream::iter(jobs).buffer_unordered(workers);

        while let Some((name, res)) = stream.next().await {
            done += 1;
            let outcome = match res {
                Ok(Outcome::Duplicate) => {
                    dups += 1;
                    "duplicate".to_string()
                }
                Ok(Outcome::Imported(p)) => {
                    imported += 1;
                    let label = if p.photo.category == CATEGORY_BIRD {
                        birds += 1;
                        "bird".to_string()
                    } else {
                        "general".to_string()
                    };
                    pending.push(*p);
                    if pending.len() >= COMMIT_CHUNK {
                        self.commit(std::mem::take(&mut pending)).await?;
                    }
                    label
                }
                Err(e) => {
                    errors += 1;
                    tracing::warn!(file = %name, error = %format!("{e:#}"), "skipping file");
                    format!("error: {e:#}")
                }
            };
            let _ = self.events.send(ImportEvent::Progress {
                batch_id,
                done,
                total,
                file: name,
                outcome,
            });
        }
        self.commit(pending).await?;

        let batch = ImportBatch {
            batch_id,
            started_at,
            finished_at: Utc::now(),
            source_path: root.display().to_string(),
            source_fs_uuid: opts.source_fs_uuid,
            files_seen: scan.files_seen as i64,
            photos_imported: imported,
            skipped_duplicates: dups,
            errors,
            bird_count: birds,
        };
        self.lake.append_import_batch(&batch).await?;
        tracing::info!(%batch_id, imported, dups, errors, birds, "import finished");
        let _ = self.events.send(ImportEvent::Finished {
            batch: batch.clone(),
        });
        Ok(batch)
    }

    /// Upload files, then record rows. Objects are written first so a row never points at a
    /// missing file; an interrupted import just leaves unreferenced objects that the next
    /// import overwrites (keys are content-addressed).
    async fn commit(&self, items: Vec<Prepared>) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let uploads = items
            .iter()
            .flat_map(|p| p.uploads.iter().cloned())
            .collect::<Vec<_>>();
        let puts: Vec<_> = uploads
            .into_iter()
            .map(|(key, data, ct)| {
                let store = self.store.clone();
                async move { store.put(&key, data, ct).await }
            })
            .collect();
        futures::stream::iter(puts)
            .buffer_unordered(self.cfg.import.upload_concurrency.max(1))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>>>()?;

        let (photos, dets): (Vec<_>, Vec<_>) =
            items.into_iter().map(|p| (p.photo, p.detections)).unzip();
        self.lake.append_photos(&photos).await?;
        self.lake
            .append_detections(&dets.into_iter().flatten().collect::<Vec<_>>())
            .await?;
        Ok(())
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn ext(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn read(path: &PathBuf) -> Result<bytes::Bytes> {
    Ok(std::fs::read(path)
        .with_context(|| format!("reading {}", path.display()))?
        .into())
}

fn prepare(
    shot: &Shot,
    batch_id: Uuid,
    import_date: NaiveDate,
    cfg: &Config,
    clock: &CameraClock,
    known: &Mutex<HashSet<String>>,
    analyzer: Option<&dyn Analyzer>,
) -> Result<Outcome> {
    let raw_bytes = shot.raw.as_ref().map(read).transpose()?;
    let jpeg_bytes = shot.jpeg.as_ref().map(read).transpose()?;
    let primary = raw_bytes
        .as_ref()
        .or(jpeg_bytes.as_ref())
        .expect("shot has a file");
    let sha = sha256_hex(primary);
    // Reserve the hash now so a duplicate later in the same batch is skipped too.
    if !known.lock().unwrap().insert(sha.clone()) {
        return Ok(Outcome::Duplicate);
    }
    let release = |e: anyhow::Error| {
        known.lock().unwrap().remove(&sha);
        e
    };
    prepare_inner(
        shot,
        &sha,
        raw_bytes,
        jpeg_bytes,
        batch_id,
        import_date,
        cfg,
        clock,
        analyzer,
    )
    .map_err(release)
}

#[allow(clippy::too_many_arguments)]
fn prepare_inner(
    shot: &Shot,
    sha: &str,
    raw_bytes: Option<bytes::Bytes>,
    jpeg_bytes: Option<bytes::Bytes>,
    batch_id: Uuid,
    import_date: NaiveDate,
    cfg: &Config,
    clock: &CameraClock,
    analyzer: Option<&dyn Analyzer>,
) -> Result<Outcome> {
    // Metadata: the JPEG twin is the most reliably parsed; the RAW fills gaps.
    let mut meta = Meta::default();
    if let Some(j) = &shot.jpeg {
        match meta::read_image_meta(j, clock) {
            Ok(m) => meta = m,
            Err(e) => tracing::debug!(file = %j.display(), error = %e, "no EXIF in JPEG"),
        }
    }
    if let Some(r) = &shot.raw {
        match meta::read_raw_meta(r, clock) {
            Ok(m) => meta.fill_from(m),
            Err(e) => tracing::debug!(file = %r.display(), error = %e, "RAW metadata unavailable"),
        }
    }

    let full = match (&jpeg_bytes, &shot.raw) {
        (Some(j), _) => render::decode_jpeg(j, meta.orientation)?,
        (None, Some(r)) => render::decode_raw_preview(r)?,
        (None, None) => unreachable!(),
    };
    let (width, height) = (full.width() as i32, full.height() as i32);

    let photo_id = Uuid::now_v7();
    let now = Utc::now();
    let findings = match analyzer {
        Some(a) => a.analyze(
            &full,
            &AnalyzeContext {
                captured_at: meta.captured_at,
                lat: meta.gps_lat,
                lon: meta.gps_lon,
            },
        )?,
        None => vec![],
    };
    let model_version = analyzer.map(|a| a.model_version());
    let detections = detections_for(photo_id, &findings, model_version.as_deref(), now);

    let rendered = render::render(full, cfg.import.preview_px, cfg.import.thumb_px)?;
    let key_date = meta
        .captured_at
        .map(|t| t.with_timezone(&Local).date_naive())
        .unwrap_or(import_date);

    let mut uploads = Vec::new();
    let raw_key = match (&shot.raw, raw_bytes) {
        (Some(p), Some(b)) => {
            let k = keys::original(key_date, sha, &ext(p));
            uploads.push((k.clone(), b, "application/octet-stream"));
            Some(k)
        }
        _ => None,
    };
    let jpeg_size = jpeg_bytes.as_ref().map(|b| b.len() as i64).unwrap_or(0);
    let jpeg_key = match (&shot.jpeg, jpeg_bytes) {
        (Some(p), Some(b)) => {
            // A paired JPEG is stored under the RAW's hash so the pair stays together.
            let k = keys::original(key_date, sha, &ext(p));
            uploads.push((k.clone(), b, "image/jpeg"));
            Some(k)
        }
        _ => None,
    };
    let raw_size = uploads
        .first()
        .filter(|_| raw_key.is_some())
        .map(|u| u.1.len() as i64)
        .unwrap_or(0);
    let preview_key = keys::preview(key_date, sha);
    let thumb_key = keys::thumb(key_date, sha);
    uploads.push((
        preview_key.clone(),
        rendered.preview_jpeg.into(),
        "image/jpeg",
    ));
    uploads.push((thumb_key.clone(), rendered.thumb_jpeg.into(), "image/jpeg"));

    let has_bird = !findings.is_empty();
    let photo = Photo {
        photo_id,
        sha256: sha.to_string(),
        import_batch_id: batch_id,
        import_date,
        imported_at: now,
        captured_at: meta.captured_at,
        original_filename: shot.display_name(),
        raw_key,
        jpeg_key,
        preview_key,
        thumb_key,
        file_size: raw_size + jpeg_size,
        camera_make: meta.camera_make,
        camera_model: meta.camera_model,
        lens: meta.lens,
        focal_mm: meta.focal_mm,
        aperture: meta.aperture,
        shutter_s: meta.shutter_s,
        iso: meta.iso,
        width: Some(width),
        height: Some(height),
        gps_lat: meta.gps_lat,
        gps_lon: meta.gps_lon,
        gps_alt: meta.gps_alt,
        place_name: match (meta.gps_lat, meta.gps_lon) {
            (Some(la), Some(lo)) => geo::place_name(la, lo),
            _ => None,
        },
        category: if has_bird {
            CATEGORY_BIRD
        } else {
            CATEGORY_GENERAL
        }
        .to_string(),
    };
    Ok(Outcome::Imported(Box::new(Prepared {
        photo,
        detections,
        uploads,
    })))
}

/// Detection rows for one ML run. A photo analyzed with no birds gets a single marker row
/// (box_index = -1) so a later re-run that finds nothing still supersedes earlier results.
pub fn detections_for(
    photo_id: Uuid,
    findings: &[photolake_ml::BirdFinding],
    model_version: Option<&str>,
    created_at: chrono::DateTime<Utc>,
) -> Vec<Detection> {
    let indices: Vec<i32> = (0..findings.len() as i32).collect();
    detections_with_indices(photo_id, findings, &indices, model_version, created_at)
}

/// [`detections_for`] with an explicit box index per finding (see `reprocess::align`).
pub fn detections_with_indices(
    photo_id: Uuid,
    findings: &[photolake_ml::BirdFinding],
    indices: &[i32],
    model_version: Option<&str>,
    created_at: chrono::DateTime<Utc>,
) -> Vec<Detection> {
    let Some(model_version) = model_version else {
        return vec![];
    };
    if findings.is_empty() {
        return vec![Detection {
            detection_id: Uuid::now_v7(),
            photo_id,
            box_index: -1,
            bbox: BBox {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
            det_score: 0.0,
            rank: 0,
            species_code: None,
            common_name: None,
            scientific_name: None,
            cls_score: None,
            final_score: None,
            model_version: model_version.to_string(),
            created_at,
        }];
    }
    let mut out = Vec::new();
    for (f, &i) in findings.iter().zip(indices) {
        let base = |rank: i32| Detection {
            detection_id: Uuid::now_v7(),
            photo_id,
            box_index: i,
            bbox: f.bbox,
            det_score: f.det_score,
            rank,
            species_code: None,
            common_name: None,
            scientific_name: None,
            cls_score: None,
            final_score: None,
            model_version: model_version.to_string(),
            created_at,
        };
        if f.candidates.is_empty() {
            out.push(base(1));
        }
        for (r, c) in f.candidates.iter().enumerate() {
            out.push(Detection {
                species_code: Some(c.species_code.clone()),
                common_name: Some(c.common_name.clone()),
                scientific_name: Some(c.scientific_name.clone()),
                cls_score: Some(c.cls_score),
                final_score: Some(c.final_score),
                ..base(r as i32 + 1)
            });
        }
    }
    out
}
