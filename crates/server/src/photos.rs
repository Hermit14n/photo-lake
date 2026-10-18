//! Browsing: photo lists with filters, photo detail, facet values for dropdowns.

use axum::Json;
use axum::extract::{Path, Query, State};
use photolake_lake::{BIRDS_MEM, Lake, PHOTOS_MEM, sql_str};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{ApiError, ApiResult};
use crate::state::Shared;

#[derive(Debug, Default, Deserialize)]
pub struct Filters {
    pub category: Option<String>,
    pub species: Option<String>,
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub place: Option<String>,
    pub tag: Option<String>,
    /// Inclusive local dates, YYYY-MM-DD.
    pub from: Option<String>,
    pub to: Option<String>,
    pub year: Option<i32>,
    pub month: Option<u32>,
    pub hour: Option<u32>,
    pub undated: Option<bool>,
    pub has_location: Option<bool>,
    pub edited: Option<bool>,
    pub rating_min: Option<i32>,
    pub batch: Option<String>,
    pub q: Option<String>,
}

fn valid_date(s: &str) -> Option<&str> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .map(|_| s)
}

impl Filters {
    /// SQL WHERE clause over `photos_mem`. Every user value is escaped with `sql_str`.
    pub fn where_clause(&self) -> String {
        let mut c: Vec<String> = vec!["TRUE".into()];
        let lt = "captured_local"; // naive local wall-clock time
        if let Some(v) = &self.category {
            c.push(format!("category = {}", sql_str(v)));
        }
        if let Some(v) = &self.species {
            c.push(format!(
                "species_codes LIKE {}",
                sql_str(&format!("%,{v},%"))
            ));
        }
        if let Some(v) = &self.camera {
            c.push(format!("camera_model = {}", sql_str(v)));
        }
        if let Some(v) = &self.lens {
            c.push(format!("lens = {}", sql_str(v)));
        }
        if let Some(v) = &self.place {
            c.push(format!("place_name = {}", sql_str(v)));
        }
        if let Some(v) = &self.tag {
            c.push(format!(
                "(',' || tags || ',') LIKE {}",
                sql_str(&format!("%,{},%", v.to_lowercase()))
            ));
        }
        if let Some(d) = self.from.as_deref().and_then(valid_date) {
            c.push(format!("captured_local >= TIMESTAMP '{d} 00:00:00'"));
        }
        if let Some(d) = self.to.as_deref().and_then(valid_date) {
            c.push(format!(
                "captured_local < TIMESTAMP '{d} 00:00:00' + INTERVAL '1 day'"
            ));
        }
        if let Some(y) = self.year {
            c.push(format!("date_part('year', {lt}) = {y}"));
        }
        if let Some(m) = self.month {
            c.push(format!("date_part('month', {lt}) = {m}"));
        }
        if let Some(h) = self.hour {
            c.push(format!("date_part('hour', {lt}) = {h}"));
        }
        match self.undated {
            Some(true) => c.push("captured_at IS NULL".into()),
            Some(false) => c.push("captured_at IS NOT NULL".into()),
            None => {}
        }
        match self.has_location {
            Some(true) => c.push("gps_lat IS NOT NULL".into()),
            Some(false) => c.push("gps_lat IS NULL".into()),
            None => {}
        }
        if self.edited == Some(true) {
            c.push("edited_fields IS NOT NULL".into());
        }
        if let Some(r) = self.rating_min {
            c.push(format!("COALESCE(rating, 0) >= {r}"));
        }
        if let Some(b) = &self.batch {
            c.push(format!("import_batch_id = {}", sql_str(b)));
        }
        if let Some(q) = self.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
            let pat = sql_str(&format!("%{}%", q.to_lowercase()));
            c.push(format!(
                "(lower(original_filename) LIKE {pat} OR lower(COALESCE(note,'')) LIKE {pat}
                  OR lower(COALESCE(title,'')) LIKE {pat} OR lower(COALESCE(species,'')) LIKE {pat}
                  OR lower(COALESCE(place_name,'')) LIKE {pat} OR lower(COALESCE(tags,'')) LIKE {pat})"
            ));
        }
        c.join(" AND ")
    }
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    #[serde(flatten)]
    pub filters: Filters,
    pub sort: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

fn order_by(sort: Option<&str>) -> &'static str {
    match sort {
        Some("captured_asc") => "captured_at ASC NULLS LAST, original_filename",
        Some("imported_desc") => "imported_at DESC, original_filename",
        Some("rating_desc") => "COALESCE(rating, 0) DESC, captured_at DESC NULLS LAST",
        _ => "captured_at DESC NULLS LAST, original_filename DESC",
    }
}

pub async fn list(
    State(app): State<Shared>,
    Query(p): Query<ListParams>,
) -> ApiResult<Json<Value>> {
    app.lake.ensure_fresh().await?;
    let w = p.filters.where_clause();
    let limit = p.limit.unwrap_or(200).min(2000);
    let offset = p.offset.unwrap_or(0);
    let items = app
        .lake
        .query_json(&format!(
            "SELECT photo_id, CAST(captured_at AS VARCHAR) AS captured_at, category, species, bird_boxes,
                    width, height, rating, place_name, original_filename, title,
                    gps_lat IS NOT NULL AS has_location, edited_fields IS NOT NULL AS edited
             FROM {PHOTOS_MEM} WHERE {w}
             ORDER BY {} LIMIT {limit} OFFSET {offset}",
            order_by(p.sort.as_deref())
        ))
        .await?;
    let total = app
        .lake
        .query_json(&format!("SELECT COUNT(*) AS n FROM {PHOTOS_MEM} WHERE {w}"))
        .await?;
    Ok(Json(
        json!({ "items": items, "total": total[0]["n"], "limit": limit, "offset": offset }),
    ))
}

/// Photo IDs for the current filter, in order (for prev/next navigation and bulk edits).
pub async fn ids(State(app): State<Shared>, Query(p): Query<ListParams>) -> ApiResult<Json<Value>> {
    app.lake.ensure_fresh().await?;
    let rows = app
        .lake
        .query_json(&format!(
            "SELECT photo_id FROM {PHOTOS_MEM} WHERE {} ORDER BY {}",
            p.filters.where_clause(),
            order_by(p.sort.as_deref())
        ))
        .await?;
    Ok(Json(Value::Array(
        rows.into_iter().map(|r| r["photo_id"].clone()).collect(),
    )))
}

fn parse_id(id: &str) -> ApiResult<String> {
    uuid::Uuid::parse_str(id)
        .map(|u| u.to_string())
        .map_err(|_| ApiError::BadRequest("invalid photo id".into()))
}

pub async fn detail(State(app): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let id = parse_id(&id)?;
    app.lake.ensure_fresh().await?;
    let lit = sql_str(&id);
    let current = app
        .lake
        .query_json(&format!(
            "SELECT * EXCLUDE (captured_at, imported_at), CAST(captured_at AS VARCHAR) AS captured_at, CAST(imported_at AS VARCHAR) AS imported_at
             FROM {PHOTOS_MEM} WHERE photo_id = {lit}"
        ))
        .await?;
    let Some(current) = current.into_iter().next() else {
        return Err(ApiError::NotFound(format!("photo {id}")));
    };
    let original = app
        .lake
        .query_json(&format!(
            "SELECT * EXCLUDE (captured_at), CAST(captured_at AS VARCHAR) AS captured_at FROM {} WHERE photo_id = {lit}",
            Lake::table_name(photolake_lake::PHOTOS)
        ))
        .await?
        .into_iter()
        .next();
    let birds = app
        .lake
        .query_json(&format!(
            "SELECT * FROM {BIRDS_MEM} WHERE photo_id = {lit} ORDER BY box_index"
        ))
        .await?;
    let candidates = app
        .lake
        .query_json(&format!(
            "SELECT box_index, rank, species_code, common_name, scientific_name, cls_score, final_score,
                    det_score, model_version
             FROM det_latest WHERE photo_id = {lit} AND box_index >= 0 ORDER BY box_index, rank"
        ))
        .await?;
    let history = app
        .lake
        .query_json(&format!(
            "SELECT target, field, op, value, CAST(created_at AS VARCHAR) AS created_at
             FROM {} WHERE photo_id = {lit} ORDER BY created_at DESC, event_id DESC",
            Lake::table_name(photolake_lake::ANNOTATIONS)
        ))
        .await?;
    // User-defined fields ("custom.*") are not pivoted into the view; collect them here.
    let custom = app
        .lake
        .query_json(&format!(
            "SELECT field, op, value FROM ann_latest
             WHERE photo_id = {lit} AND target = 'photo' AND field LIKE 'custom.%'"
        ))
        .await?
        .into_iter()
        .filter(|r| r["op"] == "set")
        .map(|r| {
            let name = r["field"]
                .as_str()
                .unwrap_or_default()
                .trim_start_matches("custom.")
                .to_string();
            (name, r["value"].clone())
        })
        .collect::<serde_json::Map<_, _>>();
    Ok(Json(json!({
        "photo": current,
        "original": original,
        "birds": birds,
        "candidates": candidates,
        "custom": custom,
        "history": history,
    })))
}

/// Distinct values (with counts) for each dropdown, honoring the other active filters.
pub async fn facets(State(app): State<Shared>, Query(f): Query<Filters>) -> ApiResult<Json<Value>> {
    app.lake.ensure_fresh().await?;
    let w = f.where_clause();
    let q = |col: &str| {
        format!(
            "SELECT {col} AS value, COUNT(*) AS n FROM {PHOTOS_MEM}
             WHERE {w} AND {col} IS NOT NULL GROUP BY {col} ORDER BY n DESC, value LIMIT 500"
        )
    };
    let cameras = app.lake.query_json(&q("camera_model")).await?;
    let lenses = app.lake.query_json(&q("lens")).await?;
    let places = app.lake.query_json(&q("place_name")).await?;
    let categories = app.lake.query_json(&q("category")).await?;
    let years = app
        .lake
        .query_json(&format!(
            "SELECT CAST(date_part('year', captured_local) AS INT) AS value, COUNT(*) AS n FROM {PHOTOS_MEM}
             WHERE {w} AND captured_at IS NOT NULL GROUP BY 1 ORDER BY 1 DESC"
        ))
        .await?;
    let species = app
        .lake
        .query_json(&format!(
            "SELECT b.species_code AS value, MAX(b.common_name) AS label, COUNT(DISTINCT b.photo_id) AS n
             FROM {BIRDS_MEM} b JOIN (SELECT photo_id FROM {PHOTOS_MEM} WHERE {w}) p ON b.photo_id = p.photo_id
             WHERE b.species_code IS NOT NULL GROUP BY b.species_code ORDER BY n DESC, label LIMIT 1000"
        ))
        .await?;
    let tags = app
        .lake
        .query_json(&format!(
            "SELECT tag AS value, COUNT(*) AS n FROM (
               SELECT unnest(string_to_array(tags, ',')) AS tag FROM {PHOTOS_MEM} WHERE {w} AND tags IS NOT NULL)
             WHERE tag <> '' GROUP BY tag ORDER BY n DESC, tag LIMIT 500"
        ))
        .await?;
    Ok(Json(json!({
        "cameras": cameras, "lenses": lenses, "places": places, "categories": categories,
        "years": years, "species": species, "tags": tags,
    })))
}

#[derive(Deserialize)]
pub struct SpeciesQuery {
    pub q: Option<String>,
    pub limit: Option<usize>,
}

/// Taxonomy autocomplete: prefix matches first, then substring matches.
pub async fn species_search(
    State(app): State<Shared>,
    Query(p): Query<SpeciesQuery>,
) -> Json<Value> {
    let q = p.q.unwrap_or_default().trim().to_lowercase();
    let limit = p.limit.unwrap_or(20).min(100);
    if q.is_empty() {
        return Json(json!([]));
    }
    // Species already in the library rank first among equally good matches.
    let seen: std::collections::HashSet<String> = match app.lake.ensure_fresh().await {
        Ok(()) => app
            .lake
            .query_json(&format!(
                "SELECT DISTINCT species_code FROM {BIRDS_MEM} WHERE species_code IS NOT NULL"
            ))
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|r| r["species_code"].as_str().map(str::to_string))
            .collect(),
        Err(_) => Default::default(),
    };
    let mut scored: Vec<(u8, &photolake_ml::manifest::Species)> = app
        .species
        .iter()
        .filter_map(|s| {
            let (c, sci) = (s.common.to_lowercase(), s.sci.to_lowercase());
            if s.code == q || c == q {
                Some((0, s))
            } else if c.starts_with(&q)
                || c.split([' ', '-']).any(|w| w.starts_with(&q))
                || sci.starts_with(&q)
            {
                Some((1, s))
            } else if c.contains(&q) || sci.contains(&q) {
                Some((2, s))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by_key(|(rank, s)| (*rank, !seen.contains(&s.code), s.common.len()));
    Json(Value::Array(
        scored
            .into_iter()
            .take(limit)
            .map(|(_, s)| json!({ "code": s.code, "common": s.common, "sci": s.sci, "family": s.family }))
            .collect(),
    ))
}
