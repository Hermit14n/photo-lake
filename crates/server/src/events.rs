//! Server-sent events: import progress, backup status, and "data changed" pings.

use std::convert::Infallible;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::Stream;
use photolake_ingest::ImportEvent;
use serde::Serialize;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::state::Shared;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerEvent {
    Import {
        event: ImportEvent,
    },
    Backup {
        status: serde_json::Value,
    },
    /// Photos or edits changed; clients should refetch.
    DataChanged,
    /// Progress of a "re-run bird ML" job.
    Reprocess {
        running: bool,
        done: usize,
        total: usize,
        birds: usize,
        error: Option<String>,
    },
    /// A card or drive was detected.
    Device {
        message: String,
    },
}

pub async fn sse(State(app): State<Shared>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(app.events.subscribe()).filter_map(|msg| {
        let ev = msg.ok()?;
        Some(Ok(Event::default().json_data(ev).unwrap_or_default()))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
