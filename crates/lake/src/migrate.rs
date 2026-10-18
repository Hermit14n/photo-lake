//! Schema upgrades for existing lakes.
//!
//! Every table carries the property `photolake.schema-version`. On open, a lake older than
//! [`SCHEMA_VERSION`] is upgraded step by step; a lake newer than this build is refused, since
//! an older build could misread or miswrite it.
//!
//! Before any step runs, the catalog is copied to `data_dir/pre-upgrade/`. Iceberg data and
//! metadata files are immutable and never deleted here, so restoring that copy
//! (`photolake rollback`) returns the lake exactly to its pre-upgrade state.
//!
//! Rules for new versions (see docs/DEVELOPING.md):
//! - only add *optional* columns; never rename, retype or reuse a column;
//! - append a [`Migration`] to [`MIGRATIONS`] and bump [`SCHEMA_VERSION`];
//! - views are rebuilt on every start, so view changes need no migration.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use iceberg::spec::{PrimitiveType, Type};
use iceberg::transaction::{AddColumn, ApplyTransactionAction, Transaction};
use iceberg::{Catalog, NamespaceIdent, TableIdent};
use sqlx::Connection;

use crate::schema::{self, NAMESPACE};

/// Schema version written by this build.
pub const SCHEMA_VERSION: u32 = 1;
pub const VERSION_PROP: &str = "photolake.schema-version";

/// One schema change.
#[derive(Debug, Clone)]
pub enum Step {
    /// Add an optional column (existing rows read it as NULL).
    AddColumn {
        table: &'static str,
        column: &'static str,
        ty: PrimitiveType,
    },
}

/// The steps that take a lake from `to - 1` to `to`.
#[derive(Debug, Clone)]
pub struct Migration {
    pub to: u32,
    pub description: &'static str,
    pub steps: Vec<Step>,
}

/// Every upgrade, oldest first. Example for a future v2:
///
/// ```ignore
/// Migration {
///     to: 2,
///     description: "photos.weather",
///     steps: vec![Step::AddColumn { table: PHOTOS, column: "weather", ty: PrimitiveType::String }],
/// }
/// ```
pub fn migrations() -> Vec<Migration> {
    vec![]
}

fn ident(table: &str) -> TableIdent {
    TableIdent::new(
        NamespaceIdent::new(NAMESPACE.to_string()),
        table.to_string(),
    )
}

/// Version recorded on a table; tables from before versioning count as 1.
async fn table_version(catalog: &dyn Catalog, table: &str) -> Result<u32> {
    let t = catalog.load_table(&ident(table)).await?;
    Ok(t.metadata()
        .properties()
        .get(VERSION_PROP)
        .and_then(|v| v.parse().ok())
        .unwrap_or(1))
}

/// The lake's version: the lowest of its tables (a half-applied upgrade resumes from there).
pub async fn lake_version(catalog: &dyn Catalog) -> Result<(u32, u32)> {
    let mut lo = u32::MAX;
    let mut hi = 0;
    for t in schema::ALL_TABLES {
        let v = table_version(catalog, t).await?;
        lo = lo.min(v);
        hi = hi.max(v);
    }
    Ok((lo, hi))
}

/// Copy the catalog database consistently (SQLite `VACUUM INTO`).
pub async fn snapshot_catalog(db: &Path, out: &Path) -> Result<()> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(out);
    let mut conn = sqlx::SqliteConnection::connect(&format!("sqlite://{}", db.display())).await?;
    let path = out.display().to_string().replace('\'', "''");
    sqlx::query(&format!("VACUUM INTO '{path}'"))
        .execute(&mut conn)
        .await
        .context("snapshotting catalog")?;
    Ok(())
}

/// Bring the lake up to `target` using `migrations`. Returns the snapshot taken, if any.
pub async fn upgrade(
    catalog: &Arc<dyn Catalog>,
    catalog_db: &Path,
    snapshot_dir: &Path,
    target: u32,
    migrations: &[Migration],
) -> Result<Option<PathBuf>> {
    let (current, newest) = lake_version(catalog.as_ref()).await?;
    if newest > target {
        bail!(
            "this library was upgraded by a newer Photolake (schema v{newest}; this build \
             understands up to v{target}).\nInstall the newer version again, or restore the \
             pre-upgrade catalog with `photolake rollback`."
        );
    }
    if current >= target {
        return Ok(None);
    }
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let snap = snapshot_dir.join(format!("catalog-v{current}-{stamp}.db"));
    snapshot_catalog(catalog_db, &snap).await?;
    tracing::info!(from = current, to = target, snapshot = %snap.display(), "upgrading lake schema");

    for m in migrations
        .iter()
        .filter(|m| m.to > current && m.to <= target)
    {
        tracing::info!(version = m.to, "{}", m.description);
        for step in &m.steps {
            apply(catalog, step, m.to).await?;
        }
    }
    // Stamp every table (including ones no step touched).
    for t in schema::ALL_TABLES {
        if table_version(catalog.as_ref(), t).await? < target {
            set_version(catalog, t, target).await?;
        }
    }
    Ok(Some(snap))
}

async fn apply(catalog: &Arc<dyn Catalog>, step: &Step, version: u32) -> Result<()> {
    match step {
        Step::AddColumn { table, column, ty } => {
            let t = catalog.load_table(&ident(table)).await?;
            if table_version(catalog.as_ref(), table).await? >= version {
                return Ok(()); // already applied before an interruption
            }
            if t.metadata()
                .current_schema()
                .field_by_name(column)
                .is_some()
            {
                tracing::info!(table, column, "column already present");
            } else {
                let tx = Transaction::new(&t);
                let tx = tx
                    .update_schema()
                    .add_column(AddColumn::optional(*column, Type::Primitive(ty.clone())))
                    .apply(tx)?;
                let t = tx
                    .commit(catalog.as_ref())
                    .await
                    .with_context(|| format!("adding {table}.{column}"))?;
                // Scans read a snapshot with that snapshot's schema, so existing data would
                // not show the new column. A data-less append makes a snapshot that carries
                // the current schema (and records the migration in the table history).
                if t.metadata().current_snapshot().is_some() {
                    let tx = Transaction::new(&t);
                    let tx = tx
                        .fast_append()
                        .set_snapshot_properties(
                            [("photolake.migration".to_string(), format!("v{version}"))].into(),
                        )
                        .apply(tx)?;
                    tx.commit(catalog.as_ref()).await?;
                }
            }
            // Stamp right away so a crash mid-upgrade doesn't re-apply this table's steps.
            set_version(catalog, table, version).await
        }
    }
}

async fn set_version(catalog: &Arc<dyn Catalog>, table: &str, version: u32) -> Result<()> {
    let t = catalog.load_table(&ident(table)).await?;
    let tx = Transaction::new(&t);
    let tx = tx
        .update_table_properties()
        .set(VERSION_PROP.to_string(), version.to_string())
        .apply(tx)?;
    tx.commit(catalog.as_ref()).await?;
    Ok(())
}

/// Pre-upgrade catalog snapshots, newest first.
pub fn list_snapshots(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "db"))
        .collect();
    v.sort();
    v.reverse();
    v
}

/// Put a snapshot back as the live catalog. The current catalog is kept beside it.
pub fn restore_snapshot(snapshot: &Path, catalog_db: &Path) -> Result<PathBuf> {
    anyhow::ensure!(snapshot.exists(), "{} not found", snapshot.display());
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let kept = catalog_db.with_file_name(format!("catalog-before-rollback-{stamp}.db"));
    if catalog_db.exists() {
        std::fs::rename(catalog_db, &kept)?;
    }
    for suffix in ["-wal", "-shm"] {
        let side = PathBuf::from(format!("{}{suffix}", catalog_db.display()));
        let _ = std::fs::remove_file(side);
    }
    std::fs::copy(snapshot, catalog_db)?;
    Ok(kept)
}
