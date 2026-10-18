//! Find camera files and pair RAW + JPEG shots.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const RAW_EXTS: &[&str] = &[
    "orf", "ori", "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "raf", "rw2", "rwl",
    "dng", "pef", "srw", "3fr", "iiq", "x3f", "erf", "mos", "kdc", "dcr",
];
pub const JPEG_EXTS: &[&str] = &["jpg", "jpeg"];

/// One shot: a RAW, a JPEG, or both.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Shot {
    pub raw: Option<PathBuf>,
    pub jpeg: Option<PathBuf>,
}

impl Shot {
    /// The file whose bytes identify the photo (RAW preferred).
    pub fn primary(&self) -> &Path {
        self.raw
            .as_deref()
            .or(self.jpeg.as_deref())
            .expect("shot has a file")
    }

    pub fn display_name(&self) -> String {
        self.primary()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

#[derive(Debug, Default)]
pub struct ScanResult {
    pub shots: Vec<Shot>,
    pub files_seen: usize,
    /// Files that are neither RAW nor JPEG (videos, sidecars, camera databases).
    pub skipped: Vec<PathBuf>,
}

fn ext_of(p: &Path) -> Option<String> {
    p.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
}

/// Where to look: `<root>/DCIM` if present, otherwise `root` itself.
pub fn media_root(root: &Path) -> PathBuf {
    let dcim = root.join("DCIM");
    if dcim.is_dir() {
        dcim
    } else {
        root.to_path_buf()
    }
}

pub fn has_dcim(root: &Path) -> bool {
    root.join("DCIM").is_dir()
}

/// Walk `root` and group files by directory + stem, so `P1010001.ORF` and `P1010001.JPG`
/// become one shot.
pub fn scan(root: &Path) -> ScanResult {
    let mut groups: BTreeMap<(PathBuf, String), Shot> = BTreeMap::new();
    let mut res = ScanResult::default();
    for entry in walkdir::WalkDir::new(media_root(root))
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.into_path();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.starts_with('.') {
            continue; // macOS resource forks, hidden files
        }
        res.files_seen += 1;
        let Some(ext) = ext_of(&path) else {
            res.skipped.push(path);
            continue;
        };
        let is_raw = RAW_EXTS.contains(&ext.as_str());
        let is_jpeg = JPEG_EXTS.contains(&ext.as_str());
        if !is_raw && !is_jpeg {
            res.skipped.push(path);
            continue;
        }
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let shot = groups.entry((dir, stem)).or_default();
        let slot = if is_raw {
            &mut shot.raw
        } else {
            &mut shot.jpeg
        };
        if slot.is_some() {
            // Two RAWs (or JPEGs) with the same stem: keep both as separate shots.
            res.shots.push(if is_raw {
                Shot {
                    raw: Some(path),
                    jpeg: None,
                }
            } else {
                Shot {
                    raw: None,
                    jpeg: Some(path),
                }
            });
        } else {
            *slot = Some(path);
        }
    }
    res.shots.extend(groups.into_values());
    res.shots.sort_by(|a, b| a.primary().cmp(b.primary()));
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_raw_and_jpeg() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("DCIM/100OLYMP");
        std::fs::create_dir_all(&d).unwrap();
        for f in [
            "P1.ORF",
            "P1.JPG",
            "P2.JPG",
            "P3.orf",
            "clip.MOV",
            ".hidden.JPG",
        ] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        let r = scan(dir.path());
        assert_eq!(r.files_seen, 5);
        assert_eq!(r.shots.len(), 3);
        assert_eq!(r.skipped.len(), 1);
        let p1 = r
            .shots
            .iter()
            .find(|s| s.display_name() == "P1.ORF")
            .unwrap();
        assert!(p1.jpeg.is_some());
        assert!(
            r.shots
                .iter()
                .any(|s| s.raw.is_none() && s.display_name() == "P2.JPG")
        );
    }
}
