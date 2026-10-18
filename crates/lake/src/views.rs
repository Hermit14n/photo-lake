//! SQL views that apply user edits (annotation events) on top of source data.
//!
//! - `ann_latest`:    the newest event per (photo, target, field), excluding reverts.
//! - `det_latest`:    detections from each photo's most recent ML run.
//! - `birds_current`: one row per bird box, with user corrections and user-drawn boxes.
//! - `photos_current`: one row per photo with edited fields applied, plus derived columns.

/// Arrow type used for every timestamp column in the lake.
pub const TS_TYPE: &str = r#"Timestamp(µs, "+00:00")"#;

const LAKE: &str = "lake.photolake";

/// Photo fields that exist in the source table and can be overridden: (name, SQL cast).
const OVERRIDABLE: &[(&str, Cast)] = &[
    ("captured_at", Cast::Ts),
    ("camera_make", Cast::Text),
    ("camera_model", Cast::Text),
    ("lens", Cast::Text),
    ("focal_mm", Cast::Double),
    ("aperture", Cast::Double),
    ("shutter_s", Cast::Double),
    ("iso", Cast::Int),
    ("width", Cast::Int),
    ("height", Cast::Int),
    ("gps_lat", Cast::Double),
    ("gps_lon", Cast::Double),
    ("gps_alt", Cast::Double),
    ("place_name", Cast::Text),
];

/// Photo fields that only exist as annotations.
const ANNOTATION_ONLY: &[(&str, Cast)] = &[
    ("note", Cast::Text),
    ("tags", Cast::Text),
    ("rating", Cast::Int),
    ("title", Cast::Text),
];

/// Source columns that pass through unchanged.
const PASSTHROUGH: &[&str] = &[
    "photo_id",
    "sha256",
    "import_batch_id",
    "import_date",
    "imported_at",
    "original_filename",
    "raw_key",
    "jpeg_key",
    "preview_key",
    "thumb_key",
    "file_size",
];

#[derive(Clone, Copy)]
enum Cast {
    Text,
    Int,
    Double,
    Ts,
}

impl Cast {
    fn apply(self, expr: &str) -> String {
        match self {
            Cast::Text => expr.to_string(),
            Cast::Int => format!("CAST({expr} AS INT)"),
            Cast::Double => format!("CAST({expr} AS DOUBLE)"),
            Cast::Ts => format!("arrow_cast(to_timestamp_micros({expr}), '{TS_TYPE}')"),
        }
    }

    fn null(self) -> String {
        match self {
            Cast::Text => "CAST(NULL AS VARCHAR)".into(),
            Cast::Int => "CAST(NULL AS INT)".into(),
            Cast::Double => "CAST(NULL AS DOUBLE)".into(),
            Cast::Ts => format!("arrow_cast(NULL, '{TS_TYPE}')"),
        }
    }
}

/// Pivot expression: "set:<value>", "clear:" or NULL for one field.
fn pivot(field: &str) -> String {
    format!(
        "MAX(CASE WHEN field = '{field}' THEN op || ':' || COALESCE(value, '') END) AS ov_{field}"
    )
}

/// Effective value: override when present, else the original (or NULL).
fn effective(field: &str, cast: Cast, original: Option<&str>) -> String {
    let orig = original.map(str::to_string).unwrap_or_else(|| cast.null());
    format!(
        "CASE WHEN o.ov_{field} IS NULL THEN {orig} \
              WHEN o.ov_{field} LIKE 'clear:%' THEN {null} \
              ELSE {val} END AS {field}",
        null = cast.null(),
        val = cast.apply(&format!("substr(o.ov_{field}, 5)")),
    )
}

pub fn statements() -> Vec<String> {
    let mut out = Vec::new();

    out.push(format!(
        "CREATE OR REPLACE VIEW ann_latest AS
         SELECT photo_id, target, field, op, value, created_at FROM (
           SELECT a.*, ROW_NUMBER() OVER (
             PARTITION BY photo_id, target, field ORDER BY created_at DESC, event_id DESC) AS rn
           FROM {LAKE}.annotations a)
         WHERE rn = 1 AND op <> 'revert'"
    ));

    out.push(format!(
        "CREATE OR REPLACE VIEW det_latest AS
         SELECT d.* FROM {LAKE}.detections d
         JOIN (SELECT photo_id AS lp, MAX(created_at) AS lc FROM {LAKE}.detections GROUP BY photo_id) l
           ON d.photo_id = l.lp AND d.created_at = l.lc"
    ));

    let box_fields = [
        "species_code",
        "common_name",
        "scientific_name",
        "bbox",
        "removed",
        "confirmed",
    ];
    out.push(format!(
        "CREATE OR REPLACE VIEW box_ov AS
         SELECT photo_id, CAST(substr(target, 5) AS INT) AS box_index, {}
         FROM ann_latest WHERE target LIKE 'box:%'
         GROUP BY photo_id, CAST(substr(target, 5) AS INT)",
        box_fields
            .iter()
            .map(|f| pivot(f))
            .collect::<Vec<_>>()
            .join(", ")
    ));

    let bbox_part = |i: usize, col: &str| {
        format!(
            "CASE WHEN o.ov_bbox LIKE 'set:%' THEN CAST(split_part(substr(o.ov_bbox, 5), ',', {i}) AS FLOAT) \
                  ELSE d.{col} END AS {col}"
        )
    };
    let species_col = |f: &str| effective(f, Cast::Text, Some(&format!("d.{f}")));
    out.push(format!(
        "CREATE OR REPLACE VIEW birds_current AS
         SELECT COALESCE(d.photo_id, o.photo_id) AS photo_id,
                COALESCE(d.box_index, o.box_index) AS box_index,
                {bx}, {by}, {bw}, {bh},
                d.det_score,
                {code}, {common}, {sci},
                d.final_score,
                CASE WHEN o.ov_species_code LIKE 'set:%' THEN 'user'
                     WHEN d.species_code IS NOT NULL THEN 'model' END AS species_source,
                (COALESCE(o.ov_confirmed, '') = 'set:true' OR COALESCE(o.ov_species_code, '') LIKE 'set:%') AS confirmed,
                d.photo_id IS NULL AS user_added,
                d.model_version
         FROM (SELECT * FROM det_latest WHERE rank = 1 AND box_index >= 0) d
         FULL OUTER JOIN box_ov o ON d.photo_id = o.photo_id AND d.box_index = o.box_index
         WHERE COALESCE(o.ov_removed, '') <> 'set:true'
           AND (d.photo_id IS NOT NULL OR o.ov_bbox LIKE 'set:%')",
        bx = bbox_part(1, "bbox_x"),
        by = bbox_part(2, "bbox_y"),
        bw = bbox_part(3, "bbox_w"),
        bh = bbox_part(4, "bbox_h"),
        code = species_col("species_code"),
        common = species_col("common_name"),
        sci = species_col("scientific_name"),
    ));

    let photo_fields: Vec<&str> = OVERRIDABLE
        .iter()
        .chain(ANNOTATION_ONLY)
        .map(|(f, _)| *f)
        .chain(["category"])
        .collect();
    out.push(format!(
        "CREATE OR REPLACE VIEW photo_ov AS
         SELECT photo_id, {}, string_agg(field, ',' ORDER BY field) AS edited_fields
         FROM ann_latest WHERE target = 'photo' GROUP BY photo_id",
        photo_fields
            .iter()
            .map(|f| pivot(f))
            .collect::<Vec<_>>()
            .join(", ")
    ));

    let mut cols: Vec<String> = PASSTHROUGH.iter().map(|c| format!("p.{c}")).collect();
    cols.extend(
        OVERRIDABLE
            .iter()
            .map(|(f, c)| effective(f, *c, Some(&format!("p.{f}")))),
    );
    cols.extend(ANNOTATION_ONLY.iter().map(|(f, c)| effective(f, *c, None)));
    // Category: explicit user choice, else derived from the latest ML run, else the import value.
    cols.push(
        "CASE WHEN o.ov_category LIKE 'set:%' THEN substr(o.ov_category, 5)
              WHEN dp.photo_id IS NOT NULL OR bc.photo_id IS NOT NULL THEN
                   CASE WHEN COALESCE(bc.bird_boxes, 0) > 0 THEN 'bird' ELSE 'general' END
              ELSE p.category END AS category"
            .into(),
    );
    cols.push("COALESCE(bc.bird_boxes, 0) AS bird_boxes".into());
    cols.push("bc.species".into());
    cols.push("bc.species_codes".into());
    cols.push("o.edited_fields".into());
    out.push(format!(
        "CREATE OR REPLACE VIEW photos_current AS
         SELECT {}
         FROM {LAKE}.photos p
         LEFT JOIN photo_ov o ON p.photo_id = o.photo_id
         LEFT JOIN (SELECT DISTINCT photo_id FROM det_latest) dp ON p.photo_id = dp.photo_id
         LEFT JOIN (SELECT photo_id, COUNT(*) AS bird_boxes,
                           NULLIF(array_to_string(array_sort(array_agg(DISTINCT common_name)), ', '), '') AS species,
                           ',' || NULLIF(array_to_string(array_sort(array_agg(DISTINCT species_code)), ','), '') || ',' AS species_codes
                    FROM birds_current GROUP BY photo_id) bc ON p.photo_id = bc.photo_id",
        cols.join(",\n                ")
    ));

    out
}
