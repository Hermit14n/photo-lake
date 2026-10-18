//! HTTP API + web UI hosting.

mod admin;
mod analytics;
pub mod auth;
mod daemon;
mod edits;
mod error;
pub mod events;
mod images;
mod paths;
mod photos;
mod settings;
pub mod state;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use axum::Router;
use axum::routing::{get, post};
use photolake_core::{Config, store::ObjectStore};
use photolake_ingest::{ImportEvent, ImportOptions, Importer};
use photolake_lake::Lake;
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};

use crate::events::ServerEvent;
use crate::state::{AppState, Shared};

pub struct ServerParts {
    pub cfg: Config,
    pub cfg_path: Option<PathBuf>,
    pub lake: Arc<Lake>,
    pub importer: Arc<Importer>,
}

pub async fn build_state(parts: ServerParts) -> Result<Shared> {
    let store = ObjectStore::photos(&parts.cfg.storage)?;
    let backup = Arc::new(photolake_backup::BackupManager::new(&parts.cfg));
    let auth = auth::Auth::new(&parts.cfg);
    let species = load_species(&parts.cfg);
    let species_by_code = species
        .iter()
        .enumerate()
        .map(|(i, s)| (s.code.clone(), i))
        .collect();
    let (events, _) = tokio::sync::broadcast::channel(1024);
    let app = Arc::new(AppState {
        cfg: RwLock::new(parts.cfg),
        cfg_path: parts.cfg_path,
        lake: parts.lake,
        store,
        importer: parts.importer,
        keys: RwLock::new(HashMap::new()),
        species,
        species_by_code,
        events,
        backup,
        auth,
        reprocessing: Default::default(),
    });
    app.reload_keys().await?;
    forward_import_events(app.clone());
    daemon::spawn_watcher(app.clone());
    daemon::spawn_import_followups(app.clone());
    daemon::spawn_schedule(app.clone());
    Ok(app)
}

fn load_species(cfg: &Config) -> Vec<photolake_ml::manifest::Species> {
    let dir = cfg.models_dir();
    photolake_ml::manifest::Manifest::load(&dir)
        .and_then(|m| photolake_ml::manifest::load_species(&dir, &m.species))
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "species list unavailable; species pickers will be empty");
            vec![]
        })
}

/// Relay importer progress to SSE clients and refresh caches when an import finishes.
fn forward_import_events(app: Shared) {
    let mut rx = app.importer.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let finished = matches!(event, ImportEvent::Finished { .. });
                    let _ = app.events.send(ServerEvent::Import { event });
                    if finished {
                        if let Err(e) = app.reload_keys().await {
                            tracing::warn!(error = %e, "reloading photo index");
                        }
                        let _ = app.events.send(ServerEvent::DataChanged);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });
}

/// Run an import in the background (used by the API and the device watcher).
pub fn spawn_import(app: Shared, path: PathBuf, fs_uuid: Option<String>) {
    tokio::spawn(async move {
        let res = app
            .importer
            .import_path(
                &path,
                ImportOptions {
                    source_fs_uuid: fs_uuid,
                },
            )
            .await;
        if let Err(e) = res {
            tracing::error!(error = %format!("{e:#}"), path = %path.display(), "import failed");
        }
    });
}

pub fn router(app: Shared) -> Router {
    let api = Router::new()
        .route("/status", get(admin::status))
        .route("/photos", get(photos::list))
        .route("/photos/ids", get(photos::ids))
        .route("/photos/{id}", get(photos::detail))
        .route("/photos/{id}/edits", post(edits::edit_photo))
        .route("/edits/bulk", post(edits::edit_bulk))
        .route("/img/{id}/{kind}", get(images::image))
        .route("/facets", get(photos::facets))
        .route("/species", get(photos::species_search))
        .route("/analytics", get(analytics::dashboard))
        .route("/map", get(analytics::map_points))
        .route("/imports", get(admin::batches).post(admin::start_import))
        .route("/sql", post(admin::sql))
        .route("/reprocess", post(admin::start_reprocess))
        .route("/import/gpx", post(admin::import_gpx))
        .route(
            "/settings",
            get(settings::get_settings).put(settings::put_settings),
        )
        .route("/backup", get(settings::backup_status))
        .route("/backup/run", post(settings::backup_run))
        .route("/drives", get(settings::drives))
        .route("/events", get(events::sse))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        .route("/auth/password", post(auth::set_password));

    let mut router = Router::new().nest("/api", api);
    let web_dir = app.config().server.web_dir.clone().or_else(default_web_dir);
    match web_dir {
        Some(dir) if dir.join("index.html").exists() => {
            let index = ServeFile::new(dir.join("index.html"));
            router = router.fallback_service(ServeDir::new(&dir).fallback(index));
        }
        _ => tracing::warn!("web UI not built (run `npm run build` in web/); serving API only"),
    }
    router
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            auth::guard,
        ))
        .layer(axum::middleware::from_fn(auth::headers))
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024)) // large GPX tracks
        .layer(CompressionLayer::new())
        .with_state(app)
}

/// `web/dist` next to the workspace, or next to the binary.
fn default_web_dir() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/dist"),
        std::env::current_exe().ok()?.parent()?.join("web"),
    ];
    candidates
        .into_iter()
        .find(|p| p.join("index.html").exists())
}

pub async fn serve(app: Shared) -> Result<()> {
    let cfg = app.config();
    let bind = cfg.server.bind.clone();
    if !auth::password_required(&cfg) && !auth::open_access_allowed(&cfg) {
        anyhow::bail!(
            "server.bind is {bind}, which other devices can reach, but no password is set.\n\
             Set one with `photolake passwd` (or bind to 127.0.0.1)."
        );
    }
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!("photolake UI on http://{bind}");
    // SSE streams never end on their own, so give in-flight requests a few seconds, then exit.
    let stopping = Arc::new(tokio::sync::Notify::new());
    let signal = stopping.clone();
    let server = axum::serve(
        listener,
        router(app).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        signal.notify_one();
    });
    tokio::select! {
        r = server => r?,
        _ = async {
            stopping.notified().await;
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        } => {}
    }
    Ok(())
}

/// Resolve on Ctrl-C or SIGTERM (`docker stop`, `systemctl stop`) so in-flight writes finish.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
    tracing::info!("shutting down");
}
