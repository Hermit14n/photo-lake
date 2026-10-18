# Photolake

A personal photo lakehouse. Plug in your camera's card and Photolake:

1. imports every new photo (RAW+JPEG pairs become one photo, duplicates are skipped),
2. finds birds and identifies the species,
3. stores the files in S3-compatible object storage and the metadata in **Apache Iceberg** tables,
4. backs everything up to the drive (or cloud) of your choice.

A local web app lets you browse, edit any field, map and analyze the collection.

```
card mounted ─► photolake (Rust) ─┬─ ingest   EXIF, RAW/JPEG pairing, sha256 de-dup, previews
                                  ├─ ml       RT-DETR bird detector → BioCLIP 2 species → eBird prior
                                  ├─ lake     Iceberg tables (iceberg-rust + DataFusion)
                                  ├─ server   REST API + React UI on http://127.0.0.1:7878
                                  └─ backup   external drive / folder / rclone cloud, nightly + on plug-in
                 RustFS (S3) ◄─── photos bucket (files) + warehouse bucket (Iceberg)
                 SQLite      ◄─── Iceberg SQL catalog
```

## Install

**Most people:** follow [docs/INSTALL.md](docs/INSTALL.md). It's one command, and it sets up
Docker (plus the NVIDIA GPU toolkit if you have an NVIDIA card) and runs everything in containers.
Your library lives in `~/Photolake`.

```bash
curl -fsSL https://github.com/Hermit14n/photo-lake/releases/latest/download/install.sh -o install.sh
bash install.sh
```

| Topic | Read |
|---|---|
| Install, step by step | [docs/INSTALL.md](docs/INSTALL.md) |
| Updating without losing anything, and rolling back | [docs/UPDATING.md](docs/UPDATING.md) |
| Using it from your phone (home Wi-Fi or anywhere via Tailscale) | [docs/REMOTE.md](docs/REMOTE.md) |
| What's protected, and how | [docs/SECURITY.md](docs/SECURITY.md) |
| Developing, releasing, and schema-change rules | [docs/DEVELOPING.md](docs/DEVELOPING.md) |

## Turning it off and on

| You want to… | Run |
|---|---|
| Turn it off (it stays off, even after a reboot) | `photolake-ctl stop` |
| Turn it back on | `photolake-ctl start` |
| Keep it running but not start at boot | `photolake-ctl autostart off` |
| Check whether it's running, and get its address | `photolake-ctl status` |

Stopping is always safe. Each write to the lake is an atomic Iceberg commit, so an import that is
interrupted simply resumes where it left off next time; already-imported photos are skipped.

**Developers / native install (no app container):** see [docs/DEVELOPING.md](docs/DEVELOPING.md)
(`scripts/install-native.sh`).

## Commands

| Command | What it does |
|---|---|
| `photolake serve` | UI + API, card watcher, backup scheduler, nightly compaction |
| `photolake import <dir>` | Import a folder or card now (`--no-ml` to skip bird ID) |
| `photolake detect <file.jpg>` | Run the bird models on one image |
| `photolake reprocess [--all]` | Re-run bird ID on photos the current models haven't seen |
| `photolake backup [--target NAME]` | Back up now |
| `photolake restore --from DIR` | Rebuild object storage + catalog from a backup folder |
| `photolake compact` | Merge small Iceberg files |
| `photolake drives` | List mounted drives and their UUIDs |
| `photolake sql "SELECT …"` | Query the lake |
| `photolake passwd [--viewer]` | Set the sign-in password (or the read-only one) |
| `photolake rollback [--list]` | Restore the catalog saved before the last schema upgrade |
| `photolake version` | App version, lake schema version, CPU/GPU build |

In the container install, run these as `docker compose exec photolake /opt/photolake/photolake <command>`.
The everyday ones are wrapped by `photolake-ctl` (`photolake-ctl help`).

## Data model

All tables are append-only. Edits never overwrite data. Each edit is a row in `annotations`, and the
`photos_current` / `birds_current` views apply the newest edit per field. Every edit can be reverted, and the full history is kept.

| Table | Contents |
|---|---|
| `photolake.photos` | One row per photo: file keys, EXIF, GPS, place. Partitioned by `month(captured_at)` |
| `photolake.detections` | Bird boxes × top-3 species per ML run (`model_version`) |
| `photolake.annotations` | Field-level edits: notes, tags, rating, location, species corrections, drawn boxes, custom fields |
| `photolake.import_batches` | One row per import |

The tables are standard Iceberg v2. You can read them with PyIceberg, DuckDB, Spark or Trino by pointing at
the SQLite catalog (`~/.local/share/photolake/catalog.db`) and the S3 endpoint.

## Bird identification

Merlin's photo models are not publicly available, so Photolake follows the same approach with open models:

1. **Detect** birds with RT-DETR (COCO, Apache-2.0). If the full frame finds nothing, 2×2 tiles are searched for distant birds.
2. **Classify** each crop with BioCLIP 2 (MIT) against all ~11k species in the eBird taxonomy.
3. **Re-rank** by location (optional). With a free eBird API key, species not reported near the photo, or in your
   home region when there's no GPS, are down-weighted.

Guesses under 30% are flagged as uncertain in the UI. One click confirms a guess or picks another species.

## Backups

Backups cover the originals, the previews, the Iceberg warehouse, and a consistent `VACUUM INTO` snapshot of the catalog (with 30
dated copies). They are additive: nothing is ever deleted at the target, and runs are incremental. They run after each import, at the daily time, and whenever the backup drive is plugged in. Local and external-drive backups are native. Cloud targets need `rclone`.

Restore onto a new machine: `docker compose up -d rustfs && photolake restore --from /media/me/HDD/photolake-backup`.

## Notes

- Object storage is **RustFS** (S3-compatible, Apache-2.0), because the `minio/minio` image is no longer published on
  Docker Hub. Any S3 endpoint works; point `storage.endpoint` (or `PHOTOLAKE_S3_ENDPOINT`) at it.
- **GPU:** only bird detection and identification use the GPU, because that's the only stage it speeds up.
  - On an RTX 4070 Ti SUPER, the 487-photo test card took 41 s instead of about 8 min on the CPU.
  - JPEG decoding and resizing already use SIMD on the CPU, hashing and upload are limited by disk and network
    speed, and queries take milliseconds.
  - The `-cuda` image (or a native `--features cuda` build plus `scripts/fetch-onnxruntime-gpu.sh` and cuDNN 9)
    uses the GPU when one works.
  - With `ml.device = "auto"`, the default, it falls back to the CPU on its own. Settings → Bird identification
    shows which one is in use.
- Iceberg compaction swaps in a rewritten table, because iceberg-rust 0.10 has no rewrite-files commit. It runs nightly for
  tables with more than 50 files.
