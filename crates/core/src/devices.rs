//! Mounted filesystems: used to spot camera cards and backup drives.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Mount {
    pub device: String,
    pub mount_point: PathBuf,
    pub fs_type: String,
    pub uuid: Option<String>,
    pub label: Option<String>,
    pub total_bytes: Option<u64>,
    pub free_bytes: Option<u64>,
    /// Has a DCIM folder (a camera card).
    pub is_camera_card: bool,
}

const IGNORED_FS: &[&str] = &[
    "proc",
    "sysfs",
    "tmpfs",
    "devtmpfs",
    "devpts",
    "cgroup",
    "cgroup2",
    "securityfs",
    "pstore",
    "debugfs",
    "tracefs",
    "configfs",
    "fusectl",
    "mqueue",
    "hugetlbfs",
    "autofs",
    "binfmt_misc",
    "squashfs",
    "overlay",
    "nsfs",
    "bpf",
    "efivarfs",
    "ramfs",
    "fuse.portal",
    "fuse.gvfsd-fuse",
];

/// Decode the octal escapes /proc/mounts uses for spaces etc. ("\040").
fn unescape(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && i + 4 <= b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 4], 8)
        {
            out.push(v);
            i += 4;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolve a /dev/disk/by-* symlink to its device path. Falls back to resolving the link
/// text when the device node itself isn't visible (inside a container that only mounts
/// /dev/disk/by-uuid).
fn resolve_link(path: &Path) -> Option<PathBuf> {
    if let Ok(p) = std::fs::canonicalize(path) {
        return Some(p);
    }
    let target = std::fs::read_link(path).ok()?;
    let joined = path.parent()?.join(target);
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    Some(out)
}

/// Mount points that belong to the container/OS rather than to a drive.
fn is_system_mount(mp: &str) -> bool {
    mp == "/"
        || [
            "/boot", "/snap", "/var/", "/etc/", "/proc", "/sys", "/dev", "/usr", "/opt",
        ]
        .iter()
        .any(|p| mp.starts_with(p))
        || mp == "/media"
        || ["/data", "/config", "/models", "/import"]
            .iter()
            .any(|p| mp == *p || mp.starts_with(&format!("{p}/")))
}

/// Map "/dev/sda1" → value, from a /dev/disk/by-* directory.
fn by_dir(dir: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if let Some(target) = resolve_link(&e.path()) {
                map.insert(
                    target.display().to_string(),
                    unescape(&e.file_name().to_string_lossy()),
                );
            }
        }
    }
    map
}

fn space(path: &Path) -> (Option<u64>, Option<u64>) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = CString::new(path.as_os_str().as_bytes()) else {
        return (None, None);
    };
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `st` is a properly sized out-param.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return (None, None);
    }
    let bs = st.f_frsize as u64;
    (Some(st.f_blocks as u64 * bs), Some(st.f_bavail as u64 * bs))
}

/// Real (block-device) mounts, excluding system filesystems and the root/boot volumes.
pub fn list_mounts() -> Vec<Mount> {
    let Ok(text) = std::fs::read_to_string("/proc/self/mounts") else {
        return vec![];
    };
    let uuids = by_dir("/dev/disk/by-uuid");
    let labels = by_dir("/dev/disk/by-label");
    let mut out = Vec::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(dev), Some(mp), Some(fs)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        if !dev.starts_with("/dev/") || IGNORED_FS.contains(&fs) {
            continue;
        }
        let mount_point = PathBuf::from(unescape(mp));
        if is_system_mount(&mount_point.to_string_lossy()) {
            continue;
        }
        let device = std::fs::canonicalize(dev)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| dev.to_string());
        let (total_bytes, free_bytes) = space(&mount_point);
        out.push(Mount {
            uuid: uuids.get(&device).cloned(),
            label: labels.get(&device).cloned(),
            is_camera_card: mount_point.join("DCIM").is_dir(),
            device,
            mount_point,
            fs_type: fs.to_string(),
            total_bytes,
            free_bytes,
        });
    }
    out
}

/// Where a filesystem with this UUID is mounted, if anywhere.
pub fn mount_point_for_uuid(uuid: &str) -> Option<PathBuf> {
    list_mounts()
        .into_iter()
        .find(|m| {
            m.uuid
                .as_deref()
                .is_some_and(|u| u.eq_ignore_ascii_case(uuid))
        })
        .map(|m| m.mount_point)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_links_without_device_nodes() {
        let dir = std::env::temp_dir().join(format!("pl-links-{}", std::process::id()));
        let by_uuid = dir.join("dev/disk/by-uuid");
        std::fs::create_dir_all(&by_uuid).unwrap();
        let link = by_uuid.join("ABCD-1234");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink("../../sdz9", &link).unwrap();
        assert_eq!(resolve_link(&link), Some(dir.join("dev/sdz9")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unescapes_spaces() {
        assert_eq!(
            super::unescape("/media/me/My\\040Card"),
            "/media/me/My Card"
        );
        assert_eq!(super::unescape("/plain"), "/plain");
    }
}
