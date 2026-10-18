//! Requires the object store from docker-compose.yml (`docker compose up -d rustfs`).

use chrono::{NaiveDate, TimeZone, Utc};
use photolake_core::Config;
use photolake_core::model::*;
use photolake_lake::Lake;
use uuid::Uuid;

fn photo(id: Uuid, batch: Uuid) -> Photo {
    Photo {
        photo_id: id,
        sha256: format!("sha-{id}"),
        import_batch_id: batch,
        import_date: NaiveDate::from_ymd_opt(2026, 10, 17).unwrap(),
        imported_at: Utc::now(),
        captured_at: Some(Utc.with_ymd_and_hms(2026, 5, 3, 14, 30, 0).unwrap()),
        original_filename: "IMG_0001.CR3".into(),
        raw_key: Some("originals/a.cr3".into()),
        jpeg_key: Some("originals/a.jpg".into()),
        preview_key: "previews/a.jpg".into(),
        thumb_key: "thumbs/a.jpg".into(),
        file_size: 25_000_000,
        camera_make: Some("Canon".into()),
        camera_model: Some("EOS R7".into()),
        lens: Some("RF100-500mm".into()),
        focal_mm: Some(500.0),
        aperture: Some(7.1),
        shutter_s: Some(1.0 / 2000.0),
        iso: Some(800),
        width: Some(6960),
        height: Some(4640),
        gps_lat: Some(40.0),
        gps_lon: Some(-75.0),
        gps_alt: None,
        place_name: None,
        category: CATEGORY_BIRD.into(),
    }
}

fn ann(photo_id: Uuid, target: &str, field: &str, value: Option<&str>, secs: i64) -> Annotation {
    Annotation {
        event_id: Uuid::now_v7(),
        photo_id,
        target: target.into(),
        field: field.into(),
        op: if value.is_some() {
            AnnotationOp::Set
        } else {
            AnnotationOp::Clear
        },
        value: value.map(str::to_string),
        created_at: Utc::now() + chrono::Duration::seconds(secs),
    }
}

#[tokio::test]
async fn write_edit_and_read_back() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = Config::default();
    cfg.data_dir = Some(tmp.path().to_path_buf());
    // Fixed buckets; each run uses a fresh catalog, so earlier runs' files are simply unreferenced.
    cfg.storage.photos_bucket = "photolake-test-photos".into();
    cfg.storage.warehouse_bucket = "photolake-test-warehouse".into();
    photolake_core::store::ensure_buckets(&cfg.storage)
        .await
        .unwrap();

    let lake = Lake::open(&cfg).await.unwrap();
    let (p1, p2, batch) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let mut general = photo(p2, batch);
    general.sha256 = "other".into();
    general.category = CATEGORY_GENERAL.into();
    general.captured_at = None; // null partition value must work
    lake.append_photos(&[photo(p1, batch), general])
        .await
        .unwrap();

    let det = |rank: i32, code: &str, name: &str, score: f32| Detection {
        detection_id: Uuid::new_v4(),
        photo_id: p1,
        box_index: 0,
        bbox: BBox {
            x: 0.1,
            y: 0.2,
            w: 0.3,
            h: 0.4,
        },
        det_score: 0.9,
        rank,
        species_code: Some(code.into()),
        common_name: Some(name.into()),
        scientific_name: None,
        cls_score: Some(score),
        final_score: Some(score),
        model_version: "test-v1".into(),
        created_at: Utc.with_ymd_and_hms(2026, 10, 17, 0, 0, 0).unwrap(),
    };
    lake.append_detections(&[
        det(1, "norcar", "Northern Cardinal", 0.8),
        det(2, "pyrrhu", "Pyrrhuloxia", 0.1),
    ])
    .await
    .unwrap();

    lake.append_annotations(&[
        ann(p1, "photo", "gps_lat", Some("41.5"), 0),
        ann(p1, "photo", "note", Some("first edit"), 1),
        ann(p1, "photo", "note", Some("second edit"), 2),
        ann(p1, "photo", "captured_at", Some("2026-05-03T15:30:00Z"), 0),
        ann(p1, "photo", "lens", None, 0),
        ann(p1, "box:0", "species_code", Some("pyrrhu"), 0),
        ann(p1, "box:0", "common_name", Some("Pyrrhuloxia"), 0),
        ann(p2, "box:0", "bbox", Some("0.5,0.5,0.1,0.1"), 0),
        ann(p2, "box:0", "common_name", Some("American Robin"), 0),
        ann(p2, "box:0", "species_code", Some("amerob"), 0),
    ])
    .await
    .unwrap();

    assert_eq!(lake.known_hashes().await.unwrap().len(), 2);

    let rows = lake
        .query_json(&format!(
            "SELECT photo_id, gps_lat, gps_lon, note, lens, category, bird_boxes, species,
                    CAST(captured_at AS VARCHAR) AS cap, edited_fields
             FROM photos_current ORDER BY photo_id = '{p1}' DESC"
        ))
        .await
        .unwrap();
    println!("{}", serde_json::to_string_pretty(&rows).unwrap());
    let r1 = &rows[0];
    assert_eq!(r1["gps_lat"], 41.5);
    assert_eq!(r1["gps_lon"], -75.0);
    assert_eq!(r1["note"], "second edit");
    assert!(r1["lens"].is_null());
    assert_eq!(r1["category"], "bird");
    assert_eq!(r1["species"], "Pyrrhuloxia");
    assert!(r1["cap"].as_str().unwrap().contains("15:30"));
    // The general photo got a user-drawn bird box, so it becomes a bird photo.
    let r2 = &rows[1];
    assert_eq!(r2["category"], "bird");
    assert_eq!(r2["species"], "American Robin");

    let birds = lake
        .query_json("SELECT photo_id, box_index, species_code, species_source, confirmed, user_added FROM birds_current")
        .await
        .unwrap();
    println!("{}", serde_json::to_string_pretty(&birds).unwrap());
    assert_eq!(birds.len(), 2);

    // Revert the gps edit: original value comes back.
    let mut revert = ann(p1, "photo", "gps_lat", None, 10);
    revert.op = AnnotationOp::Revert;
    lake.append_annotations(&[revert]).await.unwrap();
    let rows = lake
        .query_json(&format!(
            "SELECT gps_lat FROM photos_current WHERE photo_id = '{p1}'"
        ))
        .await
        .unwrap();
    assert_eq!(rows[0]["gps_lat"], 40.0);

    // Compaction must not change what the views return.
    let before = view_snapshot(&lake).await;
    for t in [
        photolake_lake::ANNOTATIONS,
        photolake_lake::DETECTIONS,
        photolake_lake::PHOTOS,
    ] {
        let (b, a) = lake.compact(t).await.unwrap();
        println!("compacted {t}: {b} -> {a} files");
        assert!(a <= b, "{t} should not grow");
    }
    assert_eq!(before, view_snapshot(&lake).await);
    // Appends still work after the swap.
    lake.append_annotations(&[ann(p2, "photo", "note", Some("after compaction"), 20)])
        .await
        .unwrap();
    let rows = lake
        .query_json(&format!(
            "SELECT note FROM photos_current WHERE photo_id = '{p2}'"
        ))
        .await
        .unwrap();
    assert_eq!(rows[0]["note"], "after compaction");
}

async fn view_snapshot(l: &Lake) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    let q = "SELECT photo_id, gps_lat, note, category, species, edited_fields FROM photos_current ORDER BY photo_id";
    let q2 = "SELECT photo_id, box_index, species_code, confirmed FROM birds_current ORDER BY photo_id, box_index";
    (
        l.query_json(q).await.unwrap(),
        l.query_json(q2).await.unwrap(),
    )
}
