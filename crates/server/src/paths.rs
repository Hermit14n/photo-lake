//! Limits on which server-side folders the API may read from.

use std::path::{Path, PathBuf};

use photolake_core::Config;

use crate::error::{ApiResult, bad_request};

/// Folders an import may start from: removable-media roots, $HOME, and `PHOTOLAKE_IMPORT_ROOTS`
/// (colon-separated; used by the container for mounted picture folders).
pub fn import_roots(cfg: &Config) -> Vec<PathBuf> {
    let mut roots = cfg.import.mount_roots.clone();
    roots.extend(std::env::var_os("HOME").map(PathBuf::from));
    if let Ok(extra) = std::env::var("PHOTOLAKE_IMPORT_ROOTS") {
        roots.extend(
            extra
                .split(':')
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
        );
    }
    roots
        .into_iter()
        .filter(|r| r.as_os_str().len() > 1)
        .filter_map(|r| r.canonicalize().ok())
        .collect()
}

/// The canonical form of `path` if it lies under an allowed import root.
pub fn allowed_import_dir(cfg: &Config, path: &Path) -> ApiResult<PathBuf> {
    let real = path
        .canonicalize()
        .map_err(|e| bad_request(format!("{}: {e}", path.display())))?;
    if import_roots(cfg).iter().any(|r| real.starts_with(r)) {
        Ok(real)
    } else {
        Err(bad_request(format!(
            "{} is outside the folders photolake may import from (your home folder and removable drives)",
            path.display()
        )))
    }
}
