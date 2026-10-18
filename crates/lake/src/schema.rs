//! Iceberg table definitions.
//!
//! IDs are stored as UUID strings rather than Iceberg `uuid` so they stay readable in SQL
//! and in other engines (DuckDB, PyIceberg, Spark) without casts.

use iceberg::spec::{NestedField, PrimitiveType, Schema, Transform, Type, UnboundPartitionSpec};

pub const NAMESPACE: &str = "photolake";
pub const PHOTOS: &str = "photos";
pub const DETECTIONS: &str = "detections";
pub const ANNOTATIONS: &str = "annotations";
pub const IMPORT_BATCHES: &str = "import_batches";
pub const ALL_TABLES: &[&str] = &[PHOTOS, DETECTIONS, ANNOTATIONS, IMPORT_BATCHES];

#[derive(Clone, Copy)]
enum T {
    Str,
    Int,
    Long,
    Float,
    Double,
    Date,
    Ts,
}

impl T {
    fn iceberg(self) -> Type {
        Type::Primitive(match self {
            T::Str => PrimitiveType::String,
            T::Int => PrimitiveType::Int,
            T::Long => PrimitiveType::Long,
            T::Float => PrimitiveType::Float,
            T::Double => PrimitiveType::Double,
            T::Date => PrimitiveType::Date,
            T::Ts => PrimitiveType::Timestamptz,
        })
    }
}

/// (name, type, required)
type Col = (&'static str, T, bool);

const PHOTO_COLS: &[Col] = &[
    ("photo_id", T::Str, true),
    ("sha256", T::Str, true),
    ("import_batch_id", T::Str, true),
    ("import_date", T::Date, true),
    ("imported_at", T::Ts, true),
    ("captured_at", T::Ts, false),
    ("original_filename", T::Str, true),
    ("raw_key", T::Str, false),
    ("jpeg_key", T::Str, false),
    ("preview_key", T::Str, true),
    ("thumb_key", T::Str, true),
    ("file_size", T::Long, true),
    ("camera_make", T::Str, false),
    ("camera_model", T::Str, false),
    ("lens", T::Str, false),
    ("focal_mm", T::Double, false),
    ("aperture", T::Double, false),
    ("shutter_s", T::Double, false),
    ("iso", T::Int, false),
    ("width", T::Int, false),
    ("height", T::Int, false),
    ("gps_lat", T::Double, false),
    ("gps_lon", T::Double, false),
    ("gps_alt", T::Double, false),
    ("place_name", T::Str, false),
    ("category", T::Str, true),
];

const DETECTION_COLS: &[Col] = &[
    ("detection_id", T::Str, true),
    ("photo_id", T::Str, true),
    ("box_index", T::Int, true),
    ("bbox_x", T::Float, true),
    ("bbox_y", T::Float, true),
    ("bbox_w", T::Float, true),
    ("bbox_h", T::Float, true),
    ("det_score", T::Float, true),
    ("rank", T::Int, true),
    ("species_code", T::Str, false),
    ("common_name", T::Str, false),
    ("scientific_name", T::Str, false),
    ("cls_score", T::Float, false),
    ("final_score", T::Float, false),
    ("model_version", T::Str, true),
    ("created_at", T::Ts, true),
];

const ANNOTATION_COLS: &[Col] = &[
    ("event_id", T::Str, true),
    ("photo_id", T::Str, true),
    ("target", T::Str, true),
    ("field", T::Str, true),
    ("op", T::Str, true),
    ("value", T::Str, false),
    ("created_at", T::Ts, true),
];

const BATCH_COLS: &[Col] = &[
    ("batch_id", T::Str, true),
    ("started_at", T::Ts, true),
    ("finished_at", T::Ts, true),
    ("source_path", T::Str, true),
    ("source_fs_uuid", T::Str, false),
    ("files_seen", T::Long, true),
    ("photos_imported", T::Long, true),
    ("skipped_duplicates", T::Long, true),
    ("errors", T::Long, true),
    ("bird_count", T::Long, true),
];

fn build(cols: &[Col]) -> Schema {
    let fields = cols
        .iter()
        .enumerate()
        .map(|(i, (name, t, required))| {
            let id = i as i32 + 1;
            let f = if *required {
                NestedField::required(id, *name, t.iceberg())
            } else {
                NestedField::optional(id, *name, t.iceberg())
            };
            f.into()
        })
        .collect::<Vec<_>>();
    Schema::builder()
        .with_fields(fields)
        .build()
        .expect("static schema is valid")
}

fn field_id(cols: &[Col], name: &str) -> i32 {
    cols.iter().position(|c| c.0 == name).expect("known column") as i32 + 1
}

pub fn schema(table: &str) -> Schema {
    match table {
        PHOTOS => build(PHOTO_COLS),
        DETECTIONS => build(DETECTION_COLS),
        ANNOTATIONS => build(ANNOTATION_COLS),
        IMPORT_BATCHES => build(BATCH_COLS),
        other => panic!("unknown table {other}"),
    }
}

/// Photos are partitioned by capture month: a personal library has many shooting days with
/// few photos each, so daily partitions would mostly produce tiny files.
pub fn partition_spec(table: &str) -> Option<UnboundPartitionSpec> {
    match table {
        PHOTOS => Some(
            UnboundPartitionSpec::builder()
                .add_partition_field(
                    field_id(PHOTO_COLS, "captured_at"),
                    "captured_month",
                    Transform::Month,
                )
                .expect("valid partition field")
                .build(),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_build() {
        for t in ALL_TABLES {
            let s = schema(t);
            assert!(s.as_struct().fields().len() > 3);
        }
        assert!(partition_spec(PHOTOS).is_some());
    }
}
