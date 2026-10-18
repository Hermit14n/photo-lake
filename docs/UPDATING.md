# Updating Photolake (without rebuilding your library)

## For users

```bash
photolake-ctl update
```

That's it. Here is what the command does:

1. Saves the version you're running, so you can go back to it.
2. Downloads the new app image, and new bird models if there are any.
3. Restarts Photolake on the new version, using **the same library folders**. Nothing is
   re-imported or copied.
4. Waits until the new version answers. **If it doesn't start, it automatically switches back**
   to the previous version (and the previous catalog, if the new version had already upgraded it).

To go back yourself:

```bash
photolake-ctl rollback
```

## Why the library survives updates

The app is **stateless**. Everything that matters lives outside it, in three places it reattaches
to on every start:

| What | Where | Changed by updates? |
|---|---|---|
| Photo files (originals, previews, thumbnails) | `storage/` (`photos` bucket) | Never. They are content-addressed by SHA-256 and written once. |
| Iceberg tables (Parquet data + metadata) | `storage/` (`warehouse` bucket) | Only by *adding* files. Iceberg never edits a data file in place. |
| Catalog (which metadata file is current for each table) | `data/catalog.db` | Gets a new pointer when a commit lands. |
| Settings and password | `config/config.toml` | New settings get defaults; old ones keep working. |

Each container image is a whole replacement of the *app*. Your data is just mounted into it.

### Schema upgrades

Sometimes a new version needs a new column, for example `photos.weather`.

- Every table records its schema version in the Iceberg table property `photolake.schema-version`.
- On start, a newer Photolake first **saves a copy of the catalog** to
  `data/pre-upgrade/catalog-v1-<date>.db`.
- It then applies the changes in order, using Iceberg's native schema evolution.
- **Old rows are not rewritten.** They read the new column as empty.
- Iceberg never deletes the old data or metadata files. So the old app version plus that saved catalog
  gives you **exactly** the library you had before the upgrade, and that's what `rollback` restores.
  Imports or edits made *after* the upgrade are not part of that copy:
  - The photo files are still in storage, so re-importing the card brings those photos back.
  - Edits made after the upgrade are lost.
- An **older** Photolake refuses to open a library a newer version has upgraded, with a clear message. It
  never corrupts it.

### New bird models

- Models are versioned separately, as `rtdetr_r50vd+bioclip2/<n>`.
- New models apply to new imports right away.
- For existing photos, use **Settings → Bird identification → Analyze photos not yet checked**.
  - Results from older models stay in the lake, labeled with their model version.
  - **Your corrections are kept on the right bird.** A re-run matches new boxes to old ones by overlap,
    and carries forward any corrected bird the new model misses.

## Moving to a new computer

1. On the old computer, back up to your external drive (Settings → Back up now).
2. On the new computer, run the installer.
3. Restore from the drive:
   ```bash
   photolake-ctl stop
   cd ~/Photolake && docker compose run --rm photolake restore --from "/media/$USER/<drive>/photolake-backup" --force
   photolake-ctl start
   ```

## For maintainers: shipping a release

1. Update `CHANGELOG.md`, and bump `version` in `Cargo.toml` and `web/package.json`.
2. If the lake schema changes, follow the rules in [DEVELOPING.md](DEVELOPING.md#changing-the-lake-schema).
3. Tag and push:
   ```bash
   git tag v0.3.0 && git push origin v0.3.0
   ```
4. GitHub Actions builds and publishes:
   - `ghcr.io/Hermit14n/photolake:0.3.0`, `:0.3` and `:latest`
   - the matching `-cuda` images
   - a GitHub release with `install.sh`
5. Users get it with `photolake-ctl update`.

Release images are immutable. Users can pin a version with `PHOTOLAKE_TAG=0.3.0` in `~/Photolake/.env`.
