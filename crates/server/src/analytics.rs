//! Aggregations for the analytics dashboard and map. All honor the same filters as the library.

use axum::Json;
use axum::extract::{Query, State};
use photolake_lake::{BIRDS_MEM, PHOTOS_MEM};
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::photos::Filters;
use crate::state::Shared;

pub async fn dashboard(
    State(app): State<Shared>,
    Query(f): Query<Filters>,
) -> ApiResult<Json<Value>> {
    app.lake.ensure_fresh().await?;
    let w = f.where_clause();
    let p = format!("(SELECT * FROM {PHOTOS_MEM} WHERE {w})");
    // Birds restricted to the filtered photos, with a local capture time.
    let b = format!(
        "(SELECT b.*, p.captured_local FROM {BIRDS_MEM} b JOIN {p} p ON b.photo_id = p.photo_id
          WHERE b.species_code IS NOT NULL)"
    );
    let lake = &app.lake;
    let q = |sql: String| async move { lake.query_json(&sql).await };

    let summary = q(format!(
        "SELECT COUNT(*) AS photos,
                SUM(CASE WHEN category = 'bird' THEN 1 ELSE 0 END) AS bird_photos,
                SUM(CASE WHEN gps_lat IS NOT NULL THEN 1 ELSE 0 END) AS located,
                SUM(CASE WHEN captured_at IS NULL THEN 1 ELSE 0 END) AS undated,
                COUNT(DISTINCT date_trunc('day', captured_local)) AS days_shot,
                CAST(MIN(captured_local) AS VARCHAR) AS first,
                CAST(MAX(captured_local) AS VARCHAR) AS last,
                SUM(file_size) AS bytes
         FROM {p}"
    ))
    .await?;
    let species_total = q(format!(
        "SELECT COUNT(DISTINCT species_code) AS species,
                COUNT(DISTINCT CASE WHEN confirmed THEN species_code END) AS confirmed_species,
                COUNT(DISTINCT CASE WHEN confirmed OR species_source = 'user' OR final_score >= 0.3
                                    THEN species_code END) AS likely_species
         FROM {b}"
    ))
    .await?;
    let per_month = q(format!(
        "SELECT to_char(date_trunc('month', captured_local), '%Y-%m') AS month,
                COUNT(*) AS photos, SUM(CASE WHEN category = 'bird' THEN 1 ELSE 0 END) AS birds
         FROM {p} WHERE captured_local IS NOT NULL GROUP BY 1 ORDER BY 1"
    ))
    .await?;
    let per_day = q(format!(
        "SELECT to_char(date_trunc('day', captured_local), '%Y-%m-%d') AS day, COUNT(*) AS photos,
                SUM(CASE WHEN category = 'bird' THEN 1 ELSE 0 END) AS birds
         FROM {p} WHERE captured_local IS NOT NULL GROUP BY 1 ORDER BY 1"
    ))
    .await?;
    let by_hour = q(format!(
        "SELECT CAST(date_part('hour', captured_local) AS INT) AS hour, COUNT(*) AS photos,
                SUM(CASE WHEN category = 'bird' THEN 1 ELSE 0 END) AS birds
         FROM {p} WHERE captured_local IS NOT NULL GROUP BY 1 ORDER BY 1"
    ))
    .await?;
    let by_weekday = q(format!(
        "SELECT CAST(date_part('dow', captured_local) AS INT) AS dow, COUNT(*) AS photos
         FROM {p} WHERE captured_local IS NOT NULL GROUP BY 1 ORDER BY 1"
    ))
    .await?;
    let top_species = q(format!(
        "SELECT species_code AS code, MAX(common_name) AS name, MAX(scientific_name) AS sci,
                COUNT(DISTINCT photo_id) AS photos,
                SUM(CASE WHEN confirmed THEN 1 ELSE 0 END) AS confirmed,
                CAST(MIN(captured_local) AS VARCHAR) AS first_seen,
                CAST(MAX(captured_local) AS VARCHAR) AS last_seen
         FROM {b} GROUP BY species_code ORDER BY photos DESC, name LIMIT 50"
    ))
    .await?;
    let life_list = q(format!(
        "SELECT to_char(first_seen, '%Y-%m-%d') AS day, COUNT(*) AS new_species,
                SUM(COUNT(*)) OVER (ORDER BY to_char(first_seen, '%Y-%m-%d')) AS total
         FROM (SELECT species_code, MIN(captured_local) AS first_seen FROM {b}
               WHERE captured_local IS NOT NULL GROUP BY species_code)
         GROUP BY 1 ORDER BY 1"
    ))
    .await?;
    let cameras = q(format!(
        "SELECT COALESCE(camera_model, 'Unknown') AS name, COUNT(*) AS photos FROM {p} GROUP BY 1 ORDER BY 2 DESC LIMIT 20"
    ))
    .await?;
    let lenses = q(format!(
        "SELECT COALESCE(lens, 'Unknown') AS name, COUNT(*) AS photos FROM {p} GROUP BY 1 ORDER BY 2 DESC LIMIT 20"
    ))
    .await?;
    let focal = q(format!(
        "SELECT CAST(floor(focal_mm / 25) * 25 AS INT) AS bucket, COUNT(*) AS photos,
                SUM(CASE WHEN category = 'bird' THEN 1 ELSE 0 END) AS birds
         FROM {p} WHERE focal_mm IS NOT NULL GROUP BY 1 ORDER BY 1"
    ))
    .await?;
    let iso = q(format!(
        "SELECT CAST(power(2, floor(log2(CAST(iso AS DOUBLE)))) AS INT) AS bucket, COUNT(*) AS photos
         FROM {p} WHERE iso IS NOT NULL AND iso > 0 GROUP BY 1 ORDER BY 1"
    ))
    .await?;
    let places = q(format!(
        "SELECT place_name AS name, COUNT(*) AS photos, AVG(gps_lat) AS lat, AVG(gps_lon) AS lon,
                SUM(CASE WHEN category = 'bird' THEN 1 ELSE 0 END) AS birds
         FROM {p} WHERE place_name IS NOT NULL GROUP BY 1 ORDER BY 2 DESC LIMIT 30"
    ))
    .await?;
    let species_by_month = q(format!(
        "SELECT to_char(date_trunc('month', captured_local), '%Y-%m') AS month,
                COUNT(DISTINCT species_code) AS species
         FROM {b} WHERE captured_local IS NOT NULL GROUP BY 1 ORDER BY 1"
    ))
    .await?;

    let mut summary = summary.into_iter().next().unwrap_or(json!({}));
    if let (Some(obj), Some(s)) = (summary.as_object_mut(), species_total.into_iter().next()) {
        obj.insert("species".into(), s["species"].clone());
        obj.insert("confirmed_species".into(), s["confirmed_species"].clone());
        obj.insert("likely_species".into(), s["likely_species"].clone());
    }
    Ok(Json(json!({
        "summary": summary,
        "per_month": per_month,
        "per_day": per_day,
        "by_hour": by_hour,
        "by_weekday": by_weekday,
        "top_species": top_species,
        "life_list": life_list,
        "species_by_month": species_by_month,
        "cameras": cameras,
        "lenses": lenses,
        "focal": focal,
        "iso": iso,
        "places": places,
    })))
}

pub async fn map_points(
    State(app): State<Shared>,
    Query(f): Query<Filters>,
) -> ApiResult<Json<Value>> {
    app.lake.ensure_fresh().await?;
    let rows = app
        .lake
        .query_json(&format!(
            "SELECT photo_id, gps_lat AS lat, gps_lon AS lon, category, species,
                    CAST(captured_local AS VARCHAR) AS captured_local
             FROM {PHOTOS_MEM} WHERE {} AND gps_lat IS NOT NULL AND gps_lon IS NOT NULL
             ORDER BY captured_at DESC NULLS LAST LIMIT 20000",
            f.where_clause()
        ))
        .await?;
    Ok(Json(Value::Array(rows)))
}
