//! Domain types shared by ingest, ML, the lake and the API.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const CATEGORY_BIRD: &str = "bird";
pub const CATEGORY_GENERAL: &str = "general";

/// One photo. A RAW+JPEG pair from the camera is a single photo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Photo {
    pub photo_id: Uuid,
    /// sha256 of the primary file (RAW when present, otherwise the JPEG).
    pub sha256: String,
    pub import_batch_id: Uuid,
    pub import_date: NaiveDate,
    pub imported_at: DateTime<Utc>,
    pub captured_at: Option<DateTime<Utc>>,
    pub original_filename: String,
    pub raw_key: Option<String>,
    pub jpeg_key: Option<String>,
    pub preview_key: String,
    pub thumb_key: String,
    pub file_size: i64,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens: Option<String>,
    pub focal_mm: Option<f64>,
    pub aperture: Option<f64>,
    pub shutter_s: Option<f64>,
    pub iso: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub gps_lat: Option<f64>,
    pub gps_lon: Option<f64>,
    pub gps_alt: Option<f64>,
    pub place_name: Option<String>,
    /// `bird` or `general`.
    pub category: String,
}

/// Normalized bounding box: all values are fractions of the image size (0..1).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct BBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// A species guess for one detected bird. Each bird box yields `top_k` rows (rank 1..=k).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Detection {
    pub detection_id: Uuid,
    pub photo_id: Uuid,
    /// Index of the bird box within the photo.
    pub box_index: i32,
    pub bbox: BBox,
    pub det_score: f32,
    pub rank: i32,
    pub species_code: Option<String>,
    pub common_name: Option<String>,
    pub scientific_name: Option<String>,
    pub cls_score: Option<f32>,
    /// Score after location/season re-ranking (equals `cls_score` when not re-ranked).
    pub final_score: Option<f32>,
    pub model_version: String,
    pub created_at: DateTime<Utc>,
}

/// What an annotation event applies to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationOp {
    Set,
    Clear,
    Revert,
}

impl AnnotationOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Set => "set",
            Self::Clear => "clear",
            Self::Revert => "revert",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "set" => Some(Self::Set),
            "clear" => Some(Self::Clear),
            "revert" => Some(Self::Revert),
            _ => None,
        }
    }
}

/// A single field-level edit. Edits never overwrite source data; the effective value of a
/// field is the latest event for (photo, target, field), falling back to the original.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Annotation {
    pub event_id: Uuid,
    pub photo_id: Uuid,
    /// "photo", or "box:<n>" for a specific bird box (species, bbox, removed).
    pub target: String,
    pub field: String,
    pub op: AnnotationOp,
    /// Canonical text value for `set` (null for `clear`/`revert`): numbers as decimal text,
    /// timestamps as RFC 3339 UTC, tags comma-separated, bbox as "x,y,w,h", booleans "true"/"false".
    pub value: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImportBatch {
    pub batch_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub source_path: String,
    pub source_fs_uuid: Option<String>,
    pub files_seen: i64,
    pub photos_imported: i64,
    pub skipped_duplicates: i64,
    pub errors: i64,
    pub bird_count: i64,
}

/// Fields a user may edit on a photo. Identity and file pointers are deliberately absent.
pub const EDITABLE_PHOTO_FIELDS: &[&str] = &[
    "captured_at",
    "camera_make",
    "camera_model",
    "lens",
    "focal_mm",
    "aperture",
    "shutter_s",
    "iso",
    "width",
    "height",
    "gps_lat",
    "gps_lon",
    "gps_alt",
    "place_name",
    "category",
    "note",
    "tags",
    "rating",
    "title",
];

/// Fields a user may edit on a bird box (`target = "box:<n>"`).
pub const EDITABLE_BOX_FIELDS: &[&str] = &[
    "species_code",
    "common_name",
    "scientific_name",
    "bbox",
    "removed",
    "confirmed",
];

/// User-defined fields are allowed with this prefix, e.g. "custom.weather".
pub const CUSTOM_FIELD_PREFIX: &str = "custom.";

pub fn is_editable(target: &str, field: &str) -> bool {
    if target == "photo" {
        EDITABLE_PHOTO_FIELDS.contains(&field)
            || (field.starts_with(CUSTOM_FIELD_PREFIX) && field.len() > CUSTOM_FIELD_PREFIX.len())
    } else if let Some(n) = target.strip_prefix("box:") {
        n.parse::<u32>().is_ok() && EDITABLE_BOX_FIELDS.contains(&field)
    } else {
        false
    }
}

/// Content-addressed object keys, sharded by capture (or import) date.
pub mod keys {
    use chrono::NaiveDate;

    fn date_prefix(date: NaiveDate) -> String {
        date.format("%Y/%m/%d").to_string()
    }

    pub fn original(date: NaiveDate, sha256: &str, ext: &str) -> String {
        format!(
            "originals/{}/{sha256}.{}",
            date_prefix(date),
            ext.to_ascii_lowercase()
        )
    }

    pub fn preview(date: NaiveDate, sha256: &str) -> String {
        format!("previews/{}/{sha256}.jpg", date_prefix(date))
    }

    pub fn thumb(date: NaiveDate, sha256: &str) -> String {
        format!("thumbs/{}/{sha256}.jpg", date_prefix(date))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_layout() {
        let d = NaiveDate::from_ymd_opt(2026, 5, 3).unwrap();
        assert_eq!(
            keys::original(d, "abc", "CR3"),
            "originals/2026/05/03/abc.cr3"
        );
        assert_eq!(keys::thumb(d, "abc"), "thumbs/2026/05/03/abc.jpg");
    }

    #[test]
    fn editability() {
        assert!(is_editable("photo", "captured_at"));
        assert!(is_editable("photo", "custom.weather"));
        assert!(!is_editable("photo", "custom."));
        assert!(!is_editable("photo", "sha256"));
        assert!(!is_editable("photo", "photo_id"));
        assert!(is_editable("box:0", "species_code"));
        assert!(!is_editable("box:x", "species_code"));
        assert!(!is_editable("box:1", "note"));
    }
}
