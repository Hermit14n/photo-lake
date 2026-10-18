#!/bin/sh
# Copy the bundled models into /data/models unless that exact version is already there.
set -eu
dest=/data/models
if [ -f "$dest/manifest.json" ] && cmp -s /models/manifest.json "$dest/manifest.json"; then
  echo "bird models up to date"
  exit 0
fi
mkdir -p "$dest.new"
cp -a /models/. "$dest.new/"
rm -rf "$dest.old"
[ -d "$dest" ] && mv "$dest" "$dest.old"
mv "$dest.new" "$dest"
rm -rf "$dest.old"
echo "installed bird models: $(grep -o '"version"[^,]*' "$dest/manifest.json")"
