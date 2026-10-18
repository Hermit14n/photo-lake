# Changelog

## 0.2.0 (2026-10-18)

### Added
- **GPU acceleration** for bird detection and identification (NVIDIA CUDA).
  - The 487-photo test card is reprocessed in 41 s instead of about 8 min on the CPU.
  - `ml.device = "auto"` (the default) falls back to the CPU on its own.
  - Detector tiles and per-photo bird crops now run as single batches.
- **Containers.** `Dockerfile` (CPU and `-cuda` images), `docker-compose.yml` with hardened defaults, and a
  bird-model image.
- **One-command installer** for non-technical users (`scripts/install.sh`). It sets up Docker and the NVIDIA
  toolkit, and generates random storage keys.
- **`photolake-ctl`**: start, stop, status, update (with automatic rollback), autostart, wifi, tailscale,
  passwd and backup.
- **Sign-in.** An admin password plus an optional view-only password, with a Settings card to manage them.
- **Remote access.** Home Wi-Fi mode, and a Tailscale sidecar giving private HTTPS from anywhere.
- **Safe upgrades.** Versioned lake schema with automatic migrations, a catalog snapshot taken before each
  one, and `photolake rollback`. Newer libraries are refused rather than corrupted.
- **Re-run bird ML from the UI.** Corrections stay attached to the right bird across re-runs.
- New commands: `photolake passwd`, `rollback`, `version` and `health`. Environment overrides
  (`PHOTOLAKE_*`) for container use. CI, release and Dependabot workflows.

### Security
- Fixed: no authentication, plus CSRF and DNS rebinding against the local API.
- Fixed: stored XSS in map popups.
- Fixed: arbitrary server paths accepted by import and backup settings, and rclone flag injection.
- Fixed: the config file was readable by other users; it's now 0600 and written atomically.
- Added: security headers (CSP and others), and a 30 s SQL timeout.
- Fixed: `quick-xml` upgraded for RUSTSEC-2026-0194/0195 (GPX parsing).

### Fixed
- `edited_fields` order is now deterministic.
- Drive detection now works inside containers.
- Graceful shutdown on SIGTERM.

## 0.1.0 (2026-10-17)
- First version: card auto-import, Iceberg lakehouse on RustFS, bird detection and species ID, web UI with
  editing, map and analytics, GPX geotagging, backups and restore, and compaction.
