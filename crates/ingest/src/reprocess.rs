//! Re-run bird ML over photos already in the lake (new models, or photos imported with ML off).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use futures::StreamExt;
use photolake_core::model::{BBox, Detection};
use photolake_core::{Config, store::ObjectStore};
use photolake_lake::{Lake, sql_str};
use photolake_ml::{AnalyzeContext, Analyzer, BirdFinding};
use uuid::Uuid;

use crate::meta::{self, CameraClock};
use crate::pipeline::{detections_for, detections_with_indices};
use crate::render;

/// Detections are committed in chunks of this many photos.
const CHUNK: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Only photos that have never been analyzed by the current model version.
    Missing,
    /// Every photo.
    All,
}

#[derive(Debug, serde::Deserialize)]
struct Row {
    photo_id: String,
    jpeg_key: Option<String>,
    preview_key: String,
    captured_at: Option<String>,
    gps_lat: Option<f64>,
    gps_lon: Option<f64>,
}

/// The latest ML run of a photo whose boxes the user has edited.
#[derive(Default)]
struct Previous {
    /// Rows of that run (every rank), by box index.
    rows: HashMap<i32, Vec<Detection>>,
    /// Box indices with user edits (including boxes the user drew).
    edited: HashSet<i32>,
}

#[derive(serde::Deserialize)]
struct PrevRow {
    photo_id: String,
    box_index: i32,
    bbox_x: f32,
    bbox_y: f32,
    bbox_w: f32,
    bbox_h: f32,
    det_score: f32,
    rank: i32,
    species_code: Option<String>,
    common_name: Option<String>,
    scientific_name: Option<String>,
    cls_score: Option<f32>,
    final_score: Option<f32>,
    model_version: String,
}

async fn load_previous(lake: &Lake) -> Result<HashMap<String, Previous>> {
    let ann = Lake::table_name(photolake_lake::ANNOTATIONS);
    let mut out: HashMap<String, Previous> = HashMap::new();
    for v in lake
        .query_json(&format!(
            "SELECT DISTINCT photo_id, CAST(substr(target, 5) AS INT) AS box_index
             FROM {ann} WHERE target LIKE 'box:%'"
        ))
        .await?
    {
        if let (Some(id), Some(i)) = (v["photo_id"].as_str(), v["box_index"].as_i64()) {
            out.entry(id.to_string())
                .or_default()
                .edited
                .insert(i as i32);
        }
    }
    let rows = lake
        .query_json(&format!(
            "SELECT photo_id, box_index, bbox_x, bbox_y, bbox_w, bbox_h, det_score, rank,
                    species_code, common_name, scientific_name, cls_score, final_score,
                    model_version
             FROM det_latest
             WHERE box_index >= 0
               AND photo_id IN (SELECT photo_id FROM {ann} WHERE target LIKE 'box:%')"
        ))
        .await?;
    for v in rows {
        let r: PrevRow = serde_json::from_value(v)?;
        let Ok(photo_id) = r.photo_id.parse::<Uuid>() else {
            continue;
        };
        let det = Detection {
            detection_id: Uuid::now_v7(),
            photo_id,
            box_index: r.box_index,
            bbox: BBox {
                x: r.bbox_x,
                y: r.bbox_y,
                w: r.bbox_w,
                h: r.bbox_h,
            },
            det_score: r.det_score,
            rank: r.rank,
            species_code: r.species_code,
            common_name: r.common_name,
            scientific_name: r.scientific_name,
            cls_score: r.cls_score,
            final_score: r.final_score,
            model_version: r.model_version,
            created_at: Utc::now(),
        };
        out.entry(r.photo_id)
            .or_default()
            .rows
            .entry(r.box_index)
            .or_default()
            .push(det);
    }
    Ok(out)
}

/// Box indices for a re-run's boxes, so user corrections (which reference `box:<index>`) stay
/// on the same bird: each new box takes the index of the previous box it overlaps most
/// (IoU >= 0.5); other new boxes get indices after every existing one.
///
/// Returns the index for each new box, and the edited previous boxes nothing matched; those
/// are carried into the new run so the user's work isn't lost.
pub fn align(prev: &[(i32, BBox)], edited: &HashSet<i32>, new: &[BBox]) -> (Vec<i32>, Vec<i32>) {
    let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
    for (j, n) in new.iter().enumerate() {
        for (k, (_, p)) in prev.iter().enumerate() {
            let iou = photolake_ml::detector::iou(n, p);
            if iou >= 0.5 {
                pairs.push((iou, j, k));
            }
        }
    }
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut assigned: Vec<Option<i32>> = vec![None; new.len()];
    let mut used = vec![false; prev.len()];
    for (_, j, k) in pairs {
        if assigned[j].is_none() && !used[k] {
            assigned[j] = Some(prev[k].0);
            used[k] = true;
        }
    }
    let mut next = prev
        .iter()
        .map(|(i, _)| *i)
        .chain(edited.iter().copied())
        .max()
        .map_or(0, |m| m + 1);
    let indices = assigned
        .into_iter()
        .map(|a| {
            a.unwrap_or_else(|| {
                next += 1;
                next - 1
            })
        })
        .collect();
    let carried = prev
        .iter()
        .zip(&used)
        .filter(|((i, _), used)| !**used && edited.contains(i))
        .map(|((i, _), _)| *i)
        .collect();
    (indices, carried)
}

/// Rows for a re-run of one photo, keeping user corrections attached (see [`align`]).
fn rerun_rows(
    photo_id: Uuid,
    findings: &[BirdFinding],
    prev: Option<&Previous>,
    version: &str,
    created_at: DateTime<Utc>,
) -> Vec<Detection> {
    let Some(prev) = prev.filter(|p| !p.edited.is_empty()) else {
        return detections_for(photo_id, findings, Some(version), created_at);
    };
    let old: Vec<(i32, BBox)> = prev
        .rows
        .iter()
        .filter_map(|(i, rows)| rows.first().map(|r| (*i, r.bbox)))
        .collect();
    let boxes: Vec<BBox> = findings.iter().map(|f| f.bbox).collect();
    let (indices, carried) = align(&old, &prev.edited, &boxes);
    let mut out = detections_with_indices(photo_id, findings, &indices, Some(version), created_at);
    for i in carried {
        out.extend(prev.rows[&i].iter().map(|r| Detection {
            detection_id: Uuid::now_v7(),
            created_at,
            ..r.clone()
        }));
    }
    out
}

pub async fn reprocess(
    cfg: &Config,
    lake: &Lake,
    analyzer: Arc<dyn Analyzer>,
    scope: Scope,
    progress: impl Fn(usize, usize, usize),
) -> Result<(usize, usize)> {
    let version = analyzer.model_version();
    let filter = match scope {
        Scope::All => String::new(),
        Scope::Missing => format!(
            "WHERE photo_id NOT IN (SELECT DISTINCT photo_id FROM {} WHERE model_version = {})",
            Lake::table_name(photolake_lake::DETECTIONS),
            sql_str(&version)
        ),
    };
    let rows: Vec<Row> = lake
        .query_json(&format!(
            "SELECT photo_id, jpeg_key, preview_key, CAST(captured_at AS VARCHAR) AS captured_at,
                    gps_lat, gps_lon
             FROM photos_current {filter} ORDER BY captured_at"
        ))
        .await?
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<_, _>>()?;
    let total = rows.len();
    let previous = load_previous(lake).await?;
    tracing::info!(total, %version, device = analyzer.device(), "reprocessing");

    let store = ObjectStore::photos(&cfg.storage)?;
    let clock = Arc::new(CameraClock::from_config(
        cfg.import.camera_timezone.as_deref(),
    ));
    let created_at = Utc::now();
    let jobs: Vec<_> = rows
        .into_iter()
        .map(|row| {
            let store = store.clone();
            let analyzer = analyzer.clone();
            let clock = clock.clone();
            async move {
                let res: Result<_> = async {
                    // Prefer the full-resolution JPEG (small birds need the pixels); RAW-only
                    // photos fall back to the stored preview.
                    let key = row
                        .jpeg_key
                        .clone()
                        .unwrap_or_else(|| row.preview_key.clone());
                    let bytes = store.get(&key).await?;
                    let photo_id: Uuid = row.photo_id.parse()?;
                    let ctx = AnalyzeContext {
                        captured_at: row
                            .captured_at
                            .as_deref()
                            .and_then(|s| DateTime::parse_from_rfc3339(&s.replace(' ', "T")).ok())
                            .map(|t| t.to_utc()),
                        lat: row.gps_lat,
                        lon: row.gps_lon,
                    };
                    let is_original = row.jpeg_key.is_some();
                    let findings = tokio::task::spawn_blocking(move || -> Result<_> {
                        let orientation = if is_original {
                            meta::read_image_meta_bytes(bytes.clone(), &clock)
                                .ok()
                                .and_then(|m| m.orientation)
                        } else {
                            None // previews are stored upright
                        };
                        let img = render::decode_jpeg(&bytes, orientation)?;
                        analyzer.analyze(&img, &ctx)
                    })
                    .await??;
                    Ok((photo_id, findings))
                }
                .await;
                res.with_context(|| format!("photo {}", row.photo_id))
            }
        })
        .collect();
    // Downloads and JPEG decoding overlap with inference; a GPU finishes each photo quickly,
    // so it needs more photos in flight to stay busy.
    let in_flight = if analyzer.device() == "cpu" {
        2
    } else {
        std::thread::available_parallelism().map_or(8, |n| n.get().saturating_sub(2).clamp(4, 16))
    };
    let mut stream = futures::stream::iter(jobs).buffer_unordered(in_flight);

    let (mut done, mut birds, mut errors) = (0usize, 0usize, 0usize);
    let mut pending = Vec::new();
    while let Some(res) = stream.next().await {
        done += 1;
        match res {
            Ok((photo_id, findings)) => {
                if !findings.is_empty() {
                    birds += 1;
                }
                pending.extend(rerun_rows(
                    photo_id,
                    &findings,
                    previous.get(&photo_id.to_string()),
                    &version,
                    created_at,
                ));
            }
            Err(e) => {
                errors += 1;
                tracing::warn!(error = %format!("{e:#}"), "reprocess failed");
            }
        }
        if done % CHUNK == 0 {
            lake.append_detections(&std::mem::take(&mut pending))
                .await?;
        }
        progress(done, total, birds);
    }
    lake.append_detections(&pending).await?;
    if errors > 0 {
        tracing::warn!(errors, "some photos could not be reprocessed");
    }
    Ok((done - errors, birds))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(x: f32) -> BBox {
        BBox {
            x,
            y: 0.1,
            w: 0.2,
            h: 0.2,
        }
    }

    #[test]
    fn align_keeps_edited_boxes_on_the_same_bird() {
        // Previous run: bird A at index 0 (edited), bird B at index 1.
        let prev = vec![(0, b(0.1)), (1, b(0.6))];
        let edited: HashSet<i32> = [0].into();
        // New model lists B first, then A (slightly moved), then a new bird C.
        let new = vec![b(0.61), b(0.11), b(0.35)];
        let (idx, carried) = align(&prev, &edited, &new);
        assert_eq!(idx, vec![1, 0, 2]);
        assert!(carried.is_empty());
    }

    #[test]
    fn align_carries_unmatched_edited_boxes() {
        // A user-drawn box (index 5, only in annotations) and an edited detection the new
        // model misses.
        let prev = vec![(0, b(0.1))];
        let edited: HashSet<i32> = [0, 5].into();
        let (idx, carried) = align(&prev, &edited, &[b(0.7)]);
        assert_eq!(idx, vec![6], "fresh index after every existing one");
        assert_eq!(carried, vec![0]);
    }
}
