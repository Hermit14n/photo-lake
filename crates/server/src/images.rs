//! Image bytes from object storage. Keys are content-addressed, so responses are immutable.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, header};
use axum::response::Response;

use crate::error::{ApiError, ApiResult};
use crate::state::Shared;

pub async fn image(
    State(app): State<Shared>,
    Path((id, kind)): Path<(String, String)>,
) -> ApiResult<Response> {
    let keys = app
        .keys
        .read()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or_else(|| ApiError::NotFound(format!("photo {id}")))?;
    let (key, content_type, download) = match kind.as_str() {
        "thumb" => (keys.thumb_key, "image/jpeg", false),
        "preview" => (keys.preview_key, "image/jpeg", false),
        // Full-size JPEG, or the preview for RAW-only shots.
        "full" => (
            keys.jpeg_key.unwrap_or(keys.preview_key),
            "image/jpeg",
            false,
        ),
        "raw" => (
            keys.raw_key
                .ok_or_else(|| ApiError::NotFound("no RAW file for this photo".into()))?,
            "application/octet-stream",
            true,
        ),
        "original" => match (keys.raw_key, keys.jpeg_key) {
            (Some(r), _) => (r, "application/octet-stream", true),
            (None, Some(j)) => (j, "image/jpeg", true),
            (None, None) => return Err(ApiError::NotFound("no original".into())),
        },
        other => return Err(ApiError::BadRequest(format!("unknown image kind {other}"))),
    };
    let bytes = app.store.get(&key).await?;
    let mut resp = Response::new(Body::from(bytes));
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    if download {
        let name = key.rsplit('/').next().unwrap_or("photo");
        if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")) {
            h.insert(header::CONTENT_DISPOSITION, v);
        }
    }
    Ok(resp)
}
