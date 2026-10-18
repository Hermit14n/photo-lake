#!/usr/bin/env bash
# Download Microsoft's official ONNX Runtime GPU build (CUDA 12) for GPU builds of Photolake.
# CUDA 12 runs on NVIDIA drivers >= 525; the prebuilt that `ort` downloads targets CUDA 13
# (driver >= 580), so GPU builds load this library at runtime instead.
#
# Usage: scripts/fetch-onnxruntime-gpu.sh [dest-dir]   (default: .ort/gpu)
set -euo pipefail
VERSION=1.31.0
SHA256=62d3af5269af2143b5d37494337573f1ffb8885f3ad7477ae0cb1cc79c8dedfd
NAME="onnxruntime-linux-x64-gpu_cuda12-${VERSION}"
DIR="$(cd "$(dirname "$0")/.." && pwd)"
DEST="${1:-$DIR/.ort/gpu}"

if [ -f "$DEST/lib/libonnxruntime.so.${VERSION}" ]; then
  echo "ONNX Runtime GPU ${VERSION} already in $DEST"
  exit 0
fi
mkdir -p "$DEST"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
echo "Downloading ONNX Runtime GPU ${VERSION} (CUDA 12, ~450 MB)…"
curl -fL --retry 3 -o "$TMP/ort.tgz" \
  "https://github.com/microsoft/onnxruntime/releases/download/v${VERSION}/${NAME}.tgz"
echo "${SHA256}  $TMP/ort.tgz" | sha256sum -c -
tar -xzf "$TMP/ort.tgz" -C "$TMP"
rm -rf "$DEST/lib"
mv "$TMP/${NAME}/lib" "$DEST/lib"
echo "Installed to $DEST/lib"
