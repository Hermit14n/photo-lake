//! Field-level edits, appended to the `annotations` table.

use axum::Json;
use axum::extract::{Path, State};
use chrono::{DateTime, Utc};
use photolake_core::model::{
    Annotation, AnnotationOp, CATEGORY_BIRD, CATEGORY_GENERAL, is_editable,
};
use photolake_lake::{PHOTOS_MEM, sql_str};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::error::{ApiResult, bad_request};
use crate::events::ServerEvent;
use crate::state::{AppState, Shared};

#[derive(Debug, Clone, Deserialize)]
pub struct EditIn {
    /// "photo" (default) or "box:<n>".
    #[serde(default = "photo_target")]
    pub target: String,
    pub field: String,
    /// "set" (default), "clear" or "revert".
    #[serde(default = "set_op")]
    pub op: String,
    #[serde(default)]
    pub value: Value,
}

fn photo_target() -> String {
    "photo".into()
}
fn set_op() -> String {
    "set".into()
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|f: &f64| f.is_finite())
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn boolean(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// Validate and convert a JSON value to the field's canonical text form.
pub fn canonical(target: &str, field: &str, v: &Value) -> Result<String, String> {
    let bad = |what: &str| format!("{field}: expected {what}");
    if target != "photo" {
        return match field {
            "bbox" => {
                let parts: Vec<f64> = match v {
                    Value::Array(a) => a.iter().filter_map(num).collect(),
                    Value::String(s) => {
                        s.split(',').filter_map(|p| p.trim().parse().ok()).collect()
                    }
                    _ => vec![],
                };
                let ok = parts.len() == 4
                    && parts.iter().all(|p| (0.0..=1.0).contains(p))
                    && parts[2] > 0.0
                    && parts[3] > 0.0;
                if !ok {
                    return Err(bad("[x, y, w, h] as fractions of the image"));
                }
                Ok(parts
                    .iter()
                    .map(|p| format!("{p:.5}"))
                    .collect::<Vec<_>>()
                    .join(","))
            }
            "removed" | "confirmed" => boolean(v)
                .map(|b| b.to_string())
                .ok_or_else(|| bad("true/false")),
            _ => text(v).filter(|s| !s.is_empty()).ok_or_else(|| bad("text")),
        };
    }
    match field {
        "captured_at" => {
            let s = text(v).ok_or_else(|| bad("an ISO 8601 date-time"))?;
            let t = DateTime::parse_from_rfc3339(&s)
                .map(|t| t.to_utc())
                .map_err(|_| {
                    bad("an ISO 8601 date-time with offset, e.g. 2026-05-03T07:15:00-04:00")
                })?;
            Ok(t.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string())
        }
        "focal_mm" | "aperture" | "shutter_s" | "gps_alt" => num(v)
            .filter(|n| *n >= 0.0 || field == "gps_alt")
            .map(|n| n.to_string())
            .ok_or_else(|| bad("a number")),
        "gps_lat" => num(v)
            .filter(|n| (-90.0..=90.0).contains(n))
            .map(|n| n.to_string())
            .ok_or_else(|| bad("latitude -90..90")),
        "gps_lon" => num(v)
            .filter(|n| (-180.0..=180.0).contains(n))
            .map(|n| n.to_string())
            .ok_or_else(|| bad("longitude -180..180")),
        "iso" | "width" | "height" => num(v)
            .filter(|n| *n >= 0.0 && n.fract() == 0.0)
            .map(|n| (n as i64).to_string())
            .ok_or_else(|| bad("a whole number")),
        "rating" => num(v)
            .filter(|n| (0.0..=5.0).contains(n) && n.fract() == 0.0)
            .map(|n| (n as i64).to_string())
            .ok_or_else(|| bad("0-5")),
        "category" => match text(v).as_deref() {
            Some(c @ (CATEGORY_BIRD | CATEGORY_GENERAL)) => Ok(c.to_string()),
            _ => Err(bad("\"bird\" or \"general\"")),
        },
        "tags" => {
            let raw: Vec<String> = match v {
                Value::Array(a) => a.iter().filter_map(text).collect(),
                Value::String(s) => s.split(',').map(str::to_string).collect(),
                _ => return Err(bad("a list of tags")),
            };
            let mut tags: Vec<String> = Vec::new();
            for t in raw
                .iter()
                .map(|t| t.trim().to_lowercase().replace(',', " "))
                .filter(|t| !t.is_empty())
            {
                if !tags.contains(&t) {
                    tags.push(t);
                }
            }
            Ok(tags.join(","))
        }
        _ => text(v).ok_or_else(|| bad("text")),
    }
}

/// Validate edits and expand species picks into code + names.
fn to_annotations(
    app: &AppState,
    photo_id: Uuid,
    edits: &[EditIn],
    now: DateTime<Utc>,
) -> Result<Vec<Annotation>, String> {
    let mut out = Vec::new();
    let mut push = |target: &str, field: &str, op: AnnotationOp, value: Option<String>| {
        out.push(Annotation {
            event_id: Uuid::now_v7(),
            photo_id,
            target: target.to_string(),
            field: field.to_string(),
            op,
            value,
            created_at: now,
        })
    };
    for e in edits {
        if !is_editable(&e.target, &e.field) {
            return Err(format!("{} is not editable on {}", e.field, e.target));
        }
        let op = AnnotationOp::parse(&e.op).ok_or_else(|| format!("unknown op {}", e.op))?;
        let value = match op {
            AnnotationOp::Set => Some(canonical(&e.target, &e.field, &e.value)?),
            _ => None,
        };
        if e.target != "photo" && e.field == "species_code" {
            // Keep names consistent with the chosen code.
            match (&op, value.as_deref().and_then(|c| app.species_lookup(c))) {
                (AnnotationOp::Set, Some(sp)) => {
                    push(
                        &e.target,
                        "common_name",
                        AnnotationOp::Set,
                        Some(sp.common.clone()),
                    );
                    push(
                        &e.target,
                        "scientific_name",
                        AnnotationOp::Set,
                        Some(sp.sci.clone()),
                    );
                }
                (AnnotationOp::Set, None) if !app.species.is_empty() => {
                    return Err(format!(
                        "unknown species code {}",
                        value.unwrap_or_default()
                    ));
                }
                (AnnotationOp::Set, None) => {}
                (other, _) => {
                    push(&e.target, "common_name", other.clone(), None);
                    push(&e.target, "scientific_name", other.clone(), None);
                }
            }
        }
        push(&e.target, &e.field, op, value);
    }
    Ok(out)
}

#[derive(Deserialize)]
pub struct EditBody {
    pub edits: Vec<EditIn>,
}

pub async fn edit_photo(
    State(app): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<EditBody>,
) -> ApiResult<Json<Value>> {
    let photo_id = Uuid::parse_str(&id).map_err(|_| bad_request("invalid photo id"))?;
    if !app.keys.read().unwrap().contains_key(&photo_id.to_string()) {
        return Err(crate::error::ApiError::NotFound(format!("photo {id}")));
    }
    let rows = to_annotations(&app, photo_id, &body.edits, Utc::now()).map_err(bad_request)?;
    app.lake.append_annotations(&rows).await?;
    let _ = app.events.send(ServerEvent::DataChanged);
    Ok(Json(json!({ "written": rows.len() })))
}

#[derive(Deserialize)]
pub struct BulkBody {
    pub photo_ids: Vec<String>,
    #[serde(default)]
    pub edits: Vec<EditIn>,
    /// Shift capture times by this many seconds (fixes a wrong camera clock or time zone).
    pub time_shift_seconds: Option<i64>,
    /// Tags merged into each photo's existing tags.
    #[serde(default)]
    pub add_tags: Vec<String>,
    #[serde(default)]
    pub remove_tags: Vec<String>,
}

pub async fn edit_bulk(
    State(app): State<Shared>,
    Json(body): Json<BulkBody>,
) -> ApiResult<Json<Value>> {
    if body.photo_ids.is_empty() {
        return Err(bad_request("no photos selected"));
    }
    if body.photo_ids.len() > 50_000 {
        return Err(bad_request("too many photos in one request"));
    }
    let ids: Vec<Uuid> = body
        .photo_ids
        .iter()
        .map(|s| Uuid::parse_str(s).map_err(|_| bad_request(format!("invalid photo id {s}"))))
        .collect::<Result<_, _>>()?;
    let now = Utc::now();
    let mut rows = Vec::new();
    for id in &ids {
        rows.extend(to_annotations(&app, *id, &body.edits, now).map_err(bad_request)?);
    }
    let list = ids
        .iter()
        .map(|i| sql_str(&i.to_string()))
        .collect::<Vec<_>>()
        .join(",");
    if !body.add_tags.is_empty() || !body.remove_tags.is_empty() {
        app.lake.ensure_fresh().await?;
        let norm = |v: &[String]| -> Vec<String> {
            canonical("photo", "tags", &json!(v))
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        };
        let (add, remove) = (norm(&body.add_tags), norm(&body.remove_tags));
        let current = app
            .lake
            .query_json(&format!(
                "SELECT photo_id, tags FROM {PHOTOS_MEM} WHERE photo_id IN ({list})"
            ))
            .await?;
        for r in current {
            let Some(pid) = r["photo_id"].as_str() else {
                continue;
            };
            let mut tags: Vec<String> = r["tags"]
                .as_str()
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            let before = tags.clone();
            tags.retain(|t| !remove.contains(t));
            for t in &add {
                if !tags.contains(t) {
                    tags.push(t.clone());
                }
            }
            if tags == before {
                continue;
            }
            rows.push(Annotation {
                event_id: Uuid::now_v7(),
                photo_id: Uuid::parse_str(pid)?,
                target: "photo".into(),
                field: "tags".into(),
                op: if tags.is_empty() {
                    AnnotationOp::Clear
                } else {
                    AnnotationOp::Set
                },
                value: (!tags.is_empty()).then(|| tags.join(",")),
                created_at: now,
            });
        }
    }
    if let Some(shift) = body.time_shift_seconds.filter(|s| *s != 0) {
        app.lake.ensure_fresh().await?;
        let current = app
            .lake
            .query_json(&format!(
                "SELECT photo_id, CAST(captured_at AS VARCHAR) AS captured_at FROM {PHOTOS_MEM}
                 WHERE photo_id IN ({list}) AND captured_at IS NOT NULL"
            ))
            .await?;
        for r in current {
            let (Some(pid), Some(ts)) = (r["photo_id"].as_str(), r["captured_at"].as_str()) else {
                continue;
            };
            let Ok(t) = DateTime::parse_from_rfc3339(ts) else {
                continue;
            };
            let shifted = t.to_utc() + chrono::Duration::seconds(shift);
            rows.push(Annotation {
                event_id: Uuid::now_v7(),
                photo_id: Uuid::parse_str(pid)?,
                target: "photo".into(),
                field: "captured_at".into(),
                op: AnnotationOp::Set,
                value: Some(shifted.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()),
                created_at: now,
            });
        }
    }
    app.lake.append_annotations(&rows).await?;
    let _ = app.events.send(ServerEvent::DataChanged);
    Ok(Json(json!({ "written": rows.len(), "photos": ids.len() })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_values() {
        assert_eq!(
            canonical(
                "photo",
                "tags",
                &json!(["Backyard", " feeder ", "backyard"])
            )
            .unwrap(),
            "backyard,feeder"
        );
        assert_eq!(
            canonical("photo", "gps_lat", &json!("40.5")).unwrap(),
            "40.5"
        );
        assert!(canonical("photo", "gps_lat", &json!(91)).is_err());
        assert_eq!(canonical("photo", "rating", &json!(4)).unwrap(), "4");
        assert!(canonical("photo", "rating", &json!(7)).is_err());
        assert_eq!(
            canonical("photo", "captured_at", &json!("2026-05-03T07:15:00-04:00")).unwrap(),
            "2026-05-03T11:15:00.000000Z"
        );
        assert_eq!(
            canonical("box:0", "bbox", &json!([0.1, 0.2, 0.3, 0.4])).unwrap(),
            "0.10000,0.20000,0.30000,0.40000"
        );
        assert!(canonical("box:0", "bbox", &json!([0.1, 0.2, 0.0, 0.4])).is_err());
        assert!(canonical("photo", "category", &json!("cat")).is_err());
    }
}
