//! Upgrading an existing lake keeps every row and edit; a newer lake is refused; rollback
//! restores the pre-upgrade catalog.
//! Requires the object store from docker-compose.yml (`docker compose up -d rustfs`).

use chrono::{NaiveDate, Utc};
use iceberg::spec::PrimitiveType;
use photolake_core::Config;
use photolake_core::model::*;
use photolake_lake::migrate::{self, Migration, Step};
use photolake_lake::{Lake, PHOTOS};
use uuid::Uuid;

fn config(dir: &std::path::Path) -> Config {
    let (mut cfg, _) = Config::load(Some(&dir.join("missing.toml"))).unwrap_or_default();
    cfg.data_dir = Some(dir.to_path_buf());
    cfg.storage.photos_bucket = "photolake-test-photos".into();
    cfg.storage.warehouse_bucket = "photolake-test-upgrade".into();
    cfg
}

fn photo(id: Uuid) -> Photo {
    Photo {
        photo_id: id,
        sha256: format!("sha-{id}"),
        import_batch_id: Uuid::new_v4(),
        import_date: NaiveDate::from_ymd_opt(2026, 10, 18).unwrap(),
        imported_at: Utc::now(),
        captured_at: Some(Utc::now()),
        original_filename: "P1.JPG".into(),
        raw_key: None,
        jpeg_key: Some("originals/p1.jpg".into()),
        preview_key: "previews/p1.jpg".into(),
        thumb_key: "thumbs/p1.jpg".into(),
        file_size: 1,
        camera_make: Some("OM".into()),
        camera_model: None,
        lens: None,
        focal_mm: None,
        aperture: None,
        shutter_s: None,
        iso: None,
        width: None,
        height: None,
        gps_lat: None,
        gps_lon: None,
        gps_alt: None,
        place_name: None,
        category: CATEGORY_GENERAL.into(),
    }
}

async fn snapshot(lake: &Lake) -> String {
    lake.ensure_fresh().await.unwrap();
    let rows = lake
        .query_json(
            "SELECT photo_id, camera_make, note, tags FROM photos_current ORDER BY photo_id",
        )
        .await
        .unwrap();
    serde_json::to_string(&rows).unwrap()
}

#[tokio::test]
async fn upgrade_preserves_data_and_refuses_downgrade() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    photolake_core::store::ensure_buckets(&cfg.storage)
        .await
        .unwrap();

    // A v1 lake with a photo and edits.
    let id = Uuid::new_v4();
    let before = {
        let lake = Lake::open(&cfg).await.unwrap();
        lake.append_photos(&[photo(id)]).await.unwrap();
        lake.append_annotations(&[
            Annotation {
                event_id: Uuid::now_v7(),
                photo_id: id,
                target: "photo".into(),
                field: "note".into(),
                op: AnnotationOp::Set,
                value: Some("kept across upgrades".into()),
                created_at: Utc::now(),
            },
            Annotation {
                event_id: Uuid::now_v7(),
                photo_id: id,
                target: "photo".into(),
                field: "camera_make".into(),
                op: AnnotationOp::Set,
                value: Some("OM System".into()),
                created_at: Utc::now(),
            },
        ])
        .await
        .unwrap();
        snapshot(&lake).await
    };
    assert!(before.contains("kept across upgrades") && before.contains("OM System"));

    // A hypothetical v2 adds a column.
    let v2 = vec![Migration {
        to: 2,
        description: "test: photos.weather",
        steps: vec![Step::AddColumn {
            table: PHOTOS,
            column: "weather",
            ty: PrimitiveType::String,
        }],
    }];
    let lake = Lake::open_with_migrations(&cfg, 2, &v2).await.unwrap();
    assert_eq!(
        snapshot(&lake).await,
        before,
        "rows and edits survive the upgrade"
    );
    let cols = lake
        .query_json("SELECT weather FROM lake.photolake.photos")
        .await
        .unwrap();
    assert_eq!(cols.len(), 1, "old rows read the new column as NULL");
    assert!(cols[0]["weather"].is_null());
    drop(lake);

    // Re-opening at v2 is a no-op (no second snapshot).
    let snaps = migrate::list_snapshots(&tmp.path().join("pre-upgrade"));
    assert_eq!(snaps.len(), 1);
    Lake::open_with_migrations(&cfg, 2, &v2).await.unwrap();
    assert_eq!(
        migrate::list_snapshots(&tmp.path().join("pre-upgrade")).len(),
        1
    );

    // The v1 build refuses the v2 lake instead of corrupting it.
    let err = Lake::open(&cfg)
        .await
        .err()
        .expect("newer lake must be refused");
    assert!(format!("{err:#}").contains("newer Photolake"), "{err:#}");

    // Rollback restores the v1 catalog, which the v1 build opens with identical data.
    migrate::restore_snapshot(&snaps[0], &cfg.catalog_db_path()).unwrap();
    let lake = Lake::open(&cfg).await.unwrap();
    assert_eq!(snapshot(&lake).await, before);
}
