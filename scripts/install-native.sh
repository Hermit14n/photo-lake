#!/usr/bin/env bash
# NATIVE developer install (no app container): build from source, prepare models, create the
# config/buckets/tables, and install a systemd user service. Most people want scripts/install.sh.
set -euo pipefail
DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$DIR"

echo "==> Object storage (RustFS)"
mkdir -p data/s3 data/s3-logs
docker compose -f docker-compose.yml -f compose.dev.yml up -d rustfs

echo "==> Building (release)"
cargo build --release -p photolake-cli
(cd web && npm ci --no-audit --no-fund && npm run build)

MODELS="${XDG_DATA_HOME:-$HOME/.local/share}/photolake/models"
if [ ! -f "$MODELS/manifest.json" ]; then
  echo "==> Downloading/exporting bird models (one time, ~1.5 GB)"
  python3 -m venv tools/.venv
  tools/.venv/bin/pip install -q --index-url https://download.pytorch.org/whl/cpu torch torchvision
  tools/.venv/bin/pip install -q open_clip_torch onnx onnxruntime huggingface_hub requests numpy
  tools/.venv/bin/python tools/export_models.py --out "$MODELS"
fi

echo "==> Config, buckets and Iceberg tables"
./target/release/photolake init

echo "==> systemd user service"
mkdir -p "$HOME/.config/systemd/user"
sed "s#PHOTOLAKE_DIR#$DIR#g" deploy/photolake.service > "$HOME/.config/systemd/user/photolake.service"
systemctl --user daemon-reload
systemctl --user enable --now photolake.service
echo
echo "Photolake is running: http://127.0.0.1:7878"
echo "Logs: journalctl --user -u photolake -f"
