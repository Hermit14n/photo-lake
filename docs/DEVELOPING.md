# Developing Photolake

## Native setup (no app container)

Prerequisites:
- Rust via rustup. `rust-toolchain.toml` pins the toolchain.
- Docker.
- Node 22.
- Python 3.10+, only for exporting the models.

```bash
docker compose -f docker-compose.yml -f compose.dev.yml up -d rustfs   # S3 on 127.0.0.1:9000
cargo build --release -p photolake-cli
(cd web && npm ci && npm run build)
./target/release/photolake init && ./target/release/photolake serve    # http://127.0.0.1:7878
```

`scripts/install-native.sh` does all of the above, exports the models, and installs a systemd user
service (`deploy/photolake.service`).

### GPU build (native)

```bash
scripts/fetch-onnxruntime-gpu.sh                 # Microsoft ONNX Runtime 1.31, CUDA 12 (checksum-verified)
cargo build --release -p photolake-cli --features cuda --target-dir target/cuda
```

You also need cuDNN 9 for CUDA 12 on the library path; the container image includes it. `ort`'s own
prebuilt GPU library targets CUDA 13 (NVIDIA driver 580+), which is why GPU builds load Microsoft's
CUDA 12 build at run time instead (driver 525+).

## Tests

```bash
cargo test --workspace          # integration tests need RustFS on :9000 (compose.dev.yml)
cargo clippy --workspace --all-targets -- -D warnings
cargo audit
(cd web && npx tsc --noEmit && npx vite build)
```

## Containers

```bash
docker build --target runtime-cpu  -t photolake:latest .
docker build --target runtime-cuda -t photolake:latest-cuda .
docker build -f docker/models.Dockerfile -t photolake-models:latest .
bash scripts/install.sh --from-source          # install from this checkout
```

## Changing the lake schema

The lake outlives every build, so schema changes must be additive and versioned:

1. **Never** rename, retype, reuse or remove a column, and never make an optional column required.
   Iceberg tracks columns by field id; the rules keep old data and other engines (PyIceberg, DuckDB,
   Spark) correct.
2. Add the column to the table definition in `crates/lake/src/schema.rs`. Append it to the end, and make it
   optional. Then extend the batch builder in `crates/lake/src/batch.rs`.
3. In `crates/lake/src/migrate.rs`:
   - bump `SCHEMA_VERSION`;
   - append a `Migration { to: N, steps: vec![Step::AddColumn { … }] }` to `migrations()`.
4. Views (`crates/lake/src/views.rs`) are rebuilt at every start, so view changes never need a migration.
5. Add a case to `crates/lake/tests/upgrade.rs`: build a lake at the previous version, then open it with the new
   one, and assert that rows, edits and views are unchanged.
6. Note the upgrade in `CHANGELOG.md`. Users can undo it with `photolake rollback`, which is the catalog snapshot
   taken automatically before migrating.

Config changes: every field has `#[serde(default)]`, and unknown keys are ignored, so old config files keep
working in both directions.

## Releasing

See [UPDATING.md](UPDATING.md#for-maintainers-shipping-a-release).
