//! Imports, read-only SQL, status.

use std::path::PathBuf;

use axum::Json;
use axum::extract::State;
use datafusion::execution::context::SQLOptions;
use photolake_lake::Lake;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{ApiResult, bad_request};
use crate::events::ServerEvent;
use crate::state::Shared;

pub async fn status(State(app): State<Shared>) -> ApiResult<Json<Value>> {
    let cfg = app.config();
    Ok(Json(json!({
        "photos": app.keys.read().unwrap().len(),
        "importing": app.importer.is_busy(),
        "ml_species": app.species.len(),
        "ml_device": app.importer.analyzer().map(|a| a.device().to_string()),
        "ml_models": app.importer.analyzer().map(|a| a.model_version()),
        "display_timezone": app.lake.display_tz().name(),
        "storage_endpoint": cfg.storage.endpoint,
        "catalog": cfg.catalog_db_path(),
        "version": env!("CARGO_PKG_VERSION"),
    })))
}

pub async fn batches(State(app): State<Shared>) -> ApiResult<Json<Value>> {
    let rows = app
        .lake
        .query_json(&format!(
            "SELECT batch_id, CAST(started_at AS VARCHAR) AS started_at, CAST(finished_at AS VARCHAR) AS finished_at,
                    source_path, files_seen, photos_imported, skipped_duplicates, errors, bird_count
             FROM {} ORDER BY started_at DESC LIMIT 200",
            Lake::table_name(photolake_lake::IMPORT_BATCHES)
        ))
        .await?;
    Ok(Json(Value::Array(rows)))
}

#[derive(Deserialize)]
pub struct ImportBody {
    pub path: String,
}

/// Start an import in the background; progress arrives over `/api/events`.
pub async fn start_import(
    State(app): State<Shared>,
    Json(body): Json<ImportBody>,
) -> ApiResult<Json<Value>> {
    let path = PathBuf::from(body.path.trim());
    if !path.is_dir() {
        return Err(bad_request(format!("{} is not a folder", path.display())));
    }
    let path = crate::paths::allowed_import_dir(&app.config(), &path)?;
    if app.importer.is_busy() {
        return Err(bad_request("an import is already running"));
    }
    crate::spawn_import(app.clone(), path, None);
    Ok(Json(json!({ "started": true })))
}

#[derive(Deserialize)]
pub struct ReprocessBody {
    /// Re-run every photo, not only those the current models haven't analyzed.
    #[serde(default)]
    pub all: bool,
}

/// Re-run bird ML over the library in the background; progress arrives over `/api/events`.
/// User corrections stay attached to their birds (see `reprocess::align`).
pub async fn start_reprocess(
    State(app): State<Shared>,
    Json(body): Json<ReprocessBody>,
) -> ApiResult<Json<Value>> {
    use std::sync::atomic::Ordering;
    let analyzer = app
        .importer
        .analyzer()
        .ok_or_else(|| bad_request("bird models aren't loaded (ML is off or not installed)"))?;
    if app.reprocessing.swap(true, Ordering::SeqCst) {
        return Err(bad_request("a re-run is already in progress"));
    }
    let scope = if body.all {
        photolake_ingest::reprocess::Scope::All
    } else {
        photolake_ingest::reprocess::Scope::Missing
    };
    let task_app = app.clone();
    tokio::spawn(async move {
        let app = task_app;
        let events = app.events.clone();
        let cfg = app.config();
        let res = photolake_ingest::reprocess::reprocess(
            &cfg,
            &app.lake,
            analyzer,
            scope,
            |done, total, birds| {
                if done % 10 == 0 || done == total {
                    let _ = events.send(ServerEvent::Reprocess {
                        running: true,
                        done,
                        total,
                        birds,
                        error: None,
                    });
                }
            },
        )
        .await;
        let (done, birds, error) = match res {
            Ok((n, b)) => (n, b, None),
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "re-run failed");
                (0, 0, Some(format!("{e:#}")))
            }
        };
        app.reprocessing.store(false, Ordering::SeqCst);
        let _ = app.events.send(ServerEvent::Reprocess {
            running: false,
            done,
            total: done,
            birds,
            error,
        });
        let _ = app.events.send(ServerEvent::DataChanged);
    });
    Ok(Json(json!({ "started": true })))
}

#[derive(Deserialize)]
pub struct SqlBody {
    pub query: String,
}

/// Read-only SQL over the lake (DDL, DML and SET statements are rejected).
pub async fn sql(State(app): State<Shared>, Json(body): Json<SqlBody>) -> ApiResult<Json<Value>> {
    app.lake.ensure_fresh().await?;
    let opts = SQLOptions::new()
        .with_allow_ddl(false)
        .with_allow_dml(false)
        .with_allow_statements(false);
    let started = std::time::Instant::now();
    let run = async {
        let df = app
            .lake
            .ctx()
            .sql_with_options(&body.query, opts)
            .await
            .map_err(|e| bad_request(e.to_string()))?;
        df.limit(0, Some(5000))?
            .collect()
            .await
            .map_err(|e| bad_request(e.to_string()))
    };
    let batches = tokio::time::timeout(std::time::Duration::from_secs(30), run)
        .await
        .map_err(|_| bad_request("query took longer than 30 s and was stopped"))??;
    let columns: Vec<String> = batches
        .first()
        .map(|b| {
            b.schema()
                .fields()
                .iter()
                .map(|f| f.name().clone())
                .collect()
        })
        .unwrap_or_default();
    let rows = photolake_lake::batches_to_json(&batches)?;
    Ok(Json(
        json!({ "columns": columns, "rows": rows, "elapsed_ms": started.elapsed().as_millis() }),
    ))
}

#[derive(Deserialize)]
pub struct GpxParams {
    /// Also replace locations photos already have.
    #[serde(default)]
    pub overwrite: bool,
    /// Max minutes between a photo and the nearest track point.
    pub max_gap_minutes: Option<i64>,
}

/// Geotag photos from a GPX track (request body is the GPX XML).
pub async fn import_gpx(
    State(app): State<Shared>,
    axum::extract::Query(p): axum::extract::Query<GpxParams>,
    body: String,
) -> ApiResult<Json<Value>> {
    use photolake_core::model::{Annotation, AnnotationOp};
    let track = photolake_ingest::gpx::parse(&body).map_err(|e| bad_request(format!("{e:#}")))?;
    let gap = p.max_gap_minutes.unwrap_or(10).clamp(1, 24 * 60) * 60;
    let (start, end) = (track[0].time, track[track.len() - 1].time);
    app.lake.ensure_fresh().await?;
    let rows = app
        .lake
        .query_json(&format!(
            "SELECT photo_id, CAST(captured_at AS VARCHAR) AS captured_at FROM {}
             WHERE captured_at IS NOT NULL {}",
            photolake_lake::PHOTOS_MEM,
            if p.overwrite {
                ""
            } else {
                "AND gps_lat IS NULL"
            }
        ))
        .await?;
    let now = chrono::Utc::now();
    let mut out = Vec::new();
    let mut matched = 0;
    for r in rows {
        let (Some(pid), Some(ts)) = (r["photo_id"].as_str(), r["captured_at"].as_str()) else {
            continue;
        };
        let Ok(t) = chrono::DateTime::parse_from_rfc3339(ts).map(|t| t.to_utc()) else {
            continue;
        };
        if t < start - chrono::Duration::seconds(gap) || t > end + chrono::Duration::seconds(gap) {
            continue;
        }
        let Some(pt) = photolake_ingest::gpx::locate(&track, t, gap) else {
            continue;
        };
        matched += 1;
        let photo_id = uuid::Uuid::parse_str(pid)?;
        let mut set = |field: &str, value: String| {
            out.push(Annotation {
                event_id: uuid::Uuid::now_v7(),
                photo_id,
                target: "photo".into(),
                field: field.into(),
                op: AnnotationOp::Set,
                value: Some(value),
                created_at: now,
            })
        };
        set("gps_lat", format!("{:.6}", pt.lat));
        set("gps_lon", format!("{:.6}", pt.lon));
        if let Some(e) = pt.ele {
            set("gps_alt", format!("{e:.1}"));
        }
        if let Some(place) = photolake_ingest::geo::place_name(pt.lat, pt.lon) {
            set("place_name", place);
        }
    }
    app.lake.append_annotations(&out).await?;
    if matched > 0 {
        let _ = app.events.send(crate::events::ServerEvent::DataChanged);
    }
    Ok(Json(
        json!({ "track_points": track.len(), "track_start": start, "track_end": end, "photos_located": matched }),
    ))
}
