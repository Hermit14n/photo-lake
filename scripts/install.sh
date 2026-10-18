#!/usr/bin/env bash
# Photolake installer: sets up everything in containers, with your library in ~/Photolake.
#
#   bash install.sh                 install (or repair) using the published images
#   bash install.sh --from-source   build the images on this computer instead (~15 min)
#
# Re-running it is safe: existing photos, settings and passwords are kept.
# Options for adopting an existing (native) library without moving it:
#   --home DIR        install folder (default ~/Photolake)
#   --s3-dir DIR      object-storage folder to use
#   --data-dir DIR    catalog/models folder to use
#   --config-dir DIR  folder holding config.toml
#   --no-start        set up but don't start
set -euo pipefail

# Filled in by the release workflow; override with PHOTOLAKE_REPO=owner/name.
REPO="${PHOTOLAKE_REPO:-}"
VERSION="${PHOTOLAKE_VERSION:-latest}"

HOME_DIR="$HOME/Photolake"
S3_DIR="" DATA_DIR="" CONFIG_DIR="" FROM_SOURCE=0 START=1
while [ $# -gt 0 ]; do
  case "$1" in
    --home) HOME_DIR="$2"; shift ;;
    --s3-dir) S3_DIR="$2"; shift ;;
    --data-dir) DATA_DIR="$2"; shift ;;
    --config-dir) CONFIG_DIR="$2"; shift ;;
    --from-source) FROM_SOURCE=1 ;;
    --no-start) START=0 ;;
    -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
    *) echo "Unknown option $1"; exit 1 ;;
  esac
  shift
done

bold() { printf '\n\033[1m%s\033[0m\n' "$*"; }
info() { printf '  %s\n' "$*"; }
warn() { printf '\033[33m  %s\033[0m\n' "$*"; }
die() { printf '\033[31m%s\033[0m\n' "$*" >&2; exit 1; }
ask() { # ask "Question" default(y/n)
  local a; read -r -p "  $1 [$( [ "$2" = y ] && echo Y/n || echo y/N )] " a || true
  a="${a:-$2}"; [[ "$a" =~ ^[Yy] ]]
}

[ "$(uname -s)" = Linux ] || die "Photolake's installer supports Linux only."
[ "$(id -u)" -ne 0 ] || die "Run this as your normal user (not with sudo); it asks for sudo when needed."
SRC_DIR="$(cd "$(dirname "$0")/.." 2>/dev/null && pwd || true)"
IN_CHECKOUT=0; [ -f "$SRC_DIR/docker-compose.yml" ] && [ -d "$SRC_DIR/crates" ] && IN_CHECKOUT=1
. /etc/os-release 2>/dev/null || true
DISTRO="${ID:-unknown} ${ID_LIKE:-}"

bold "Photolake installer"
info "Library folder: $HOME_DIR"

# ---------------- Docker ----------------
bold "1/6  Docker (runs Photolake's containers)"
if ! command -v docker >/dev/null 2>&1; then
  info "Docker isn't installed. It's free and installs from Docker's official script:"
  info "    curl -fsSL https://get.docker.com | sudo sh"
  ask "Install Docker now? (asks for your password)" y || die "Install Docker, then run this again."
  curl -fsSL https://get.docker.com | sudo sh
  sudo usermod -aG docker "$USER"
  bold "Docker is installed."
  info "Log out and back in (or restart the computer) so you can use Docker without sudo,"
  info "then run this installer again."
  exit 0
fi
if ! docker info >/dev/null 2>&1; then
  if id -nG "$USER" | grep -qw docker; then
    info "Starting the Docker service…"
    sudo systemctl enable --now docker
  else
    info "Your user can't use Docker yet. Adding you to the 'docker' group (asks for your password)…"
    sudo usermod -aG docker "$USER"
    die "Done. Log out and back in, then run this installer again."
  fi
fi
docker compose version >/dev/null 2>&1 || die "Docker Compose v2 is missing: sudo apt install docker-compose-plugin (or re-run get.docker.com)."
info "Docker $(docker version --format '{{.Server.Version}}') ✓"

# ---------------- GPU ----------------
bold "2/6  NVIDIA GPU (optional: identifies birds ~10x faster)"
GPU=0
if command -v nvidia-smi >/dev/null 2>&1 && nvidia-smi >/dev/null 2>&1; then
  info "Found: $(nvidia-smi --query-gpu=name,driver_version --format=csv,noheader | head -1)"
  gpu_works() { docker run --rm --gpus all nvidia/cuda:12.8.1-base-ubuntu24.04 nvidia-smi -L >/dev/null 2>&1; }
  if gpu_works; then
    GPU=1
  else
    info "Docker can't use the GPU yet; that needs NVIDIA's container toolkit."
    if ask "Install nvidia-container-toolkit now? (asks for your password)" y; then
      case "$DISTRO" in
        *ubuntu*|*debian*)
          curl -fsSL https://nvidia.github.io/libnvidia-container/gpgkey \
            | sudo gpg --yes --dearmor -o /usr/share/keyrings/nvidia-container-toolkit-keyring.gpg
          curl -fsSL https://nvidia.github.io/libnvidia-container/stable/deb/nvidia-container-toolkit.list \
            | sed 's#deb https://#deb [signed-by=/usr/share/keyrings/nvidia-container-toolkit-keyring.gpg] https://#g' \
            | sudo tee /etc/apt/sources.list.d/nvidia-container-toolkit.list >/dev/null
          sudo apt-get update -qq && sudo apt-get install -y -qq nvidia-container-toolkit ;;
        *fedora*|*rhel*|*centos*)
          curl -fsSL https://nvidia.github.io/libnvidia-container/stable/rpm/nvidia-container-toolkit.repo \
            | sudo tee /etc/yum.repos.d/nvidia-container-toolkit.repo >/dev/null
          sudo dnf install -y nvidia-container-toolkit ;;
        *) warn "Unknown distribution; see https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/latest/install-guide.html" ;;
      esac
      sudo nvidia-ctk runtime configure --runtime=docker && sudo systemctl restart docker
      gpu_works && GPU=1 || warn "The GPU still isn't usable from Docker; continuing on the CPU."
    fi
  fi
  [ "$GPU" = 1 ] && info "GPU acceleration ✓"
else
  info "No NVIDIA GPU found; bird identification will use the CPU (works fine, just slower)."
fi

# ---------------- Files ----------------
bold "3/6  Library folder"
mkdir -p "$HOME_DIR"
cd "$HOME_DIR"
# On a re-run, keep using the folders this install already points at.
prev() { [ -f .env ] && grep -E "^$1=" .env | tail -1 | cut -d= -f2- || true; }
S3_DIR="${S3_DIR:-$(prev PHOTOLAKE_S3_DATA)}"
DATA_DIR="${DATA_DIR:-$(prev PHOTOLAKE_DATA)}"
CONFIG_DIR="${CONFIG_DIR:-$(prev PHOTOLAKE_CONFIG_DIR)}"
S3_DIR="${S3_DIR:-$HOME_DIR/storage}"
DATA_DIR="${DATA_DIR:-$HOME_DIR/data}"
CONFIG_DIR="${CONFIG_DIR:-$HOME_DIR/config}"
mkdir -p "$S3_DIR" "$S3_DIR-logs" "$DATA_DIR" "$CONFIG_DIR" "$HOME_DIR/import" "$DATA_DIR/home"
chmod 700 "$HOME_DIR" "$DATA_DIR" "$CONFIG_DIR" "$S3_DIR"

# App files (compose definitions, control script).
APP_FILES=(docker-compose.yml compose.gpu.yml docker/tailscale-serve.json scripts/photolake-ctl)
if [ "$IN_CHECKOUT" = 1 ] && [ "$SRC_DIR" != "$HOME_DIR" ]; then
  for f in "${APP_FILES[@]}"; do mkdir -p "$(dirname "$f")"; cp "$SRC_DIR/$f" "$f"; done
elif [ "$IN_CHECKOUT" = 0 ]; then
  [ -n "$REPO" ] || die "Set PHOTOLAKE_REPO=owner/name (the GitHub repository to install from)."
  ref="$VERSION"; [ "$ref" = latest ] && ref="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p')"
  [ -n "$ref" ] || die "Couldn't find a release of $REPO."
  for f in "${APP_FILES[@]}"; do
    mkdir -p "$(dirname "$f")"
    curl -fsSL "https://raw.githubusercontent.com/$REPO/$ref/$f" -o "$f"
  done
  VERSION="${ref#v}"
fi
chmod +x scripts/photolake-ctl

# Settings for the containers (.env). Existing values win, so re-running never rotates keys.
touch .env; chmod 600 .env
env_get() { grep -E "^$1=" .env | tail -1 | cut -d= -f2- || true; }
env_default() { [ -n "$(env_get "$1")" ] || printf '%s=%s\n' "$1" "$2" >> .env; }
rand() { head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n' | head -c "$1"; }
TZ_NAME="$(timedatectl show -p Timezone --value 2>/dev/null || cat /etc/timezone 2>/dev/null || echo UTC)"
IMAGE="photolake"; MODELS_IMAGE="photolake-models"; TAG="latest"
if [ "$FROM_SOURCE" = 0 ] && [ -n "$REPO" ]; then
  owner="$(echo "${REPO%%/*}" | tr '[:upper:]' '[:lower:]')"
  IMAGE="ghcr.io/$owner/photolake"; MODELS_IMAGE="ghcr.io/$owner/photolake-models"
  [ "$VERSION" != latest ] && TAG="$VERSION"
fi
env_default PUID "$(id -u)"
env_default PGID "$(id -g)"
env_default HOST_USER "$USER"
env_default TZ "$TZ_NAME"
env_default PHOTOLAKE_IMAGE "$IMAGE"
env_default PHOTOLAKE_TAG "$TAG"
env_default PHOTOLAKE_MODELS_IMAGE "$MODELS_IMAGE"
env_default PHOTOLAKE_MODELS_TAG latest
env_default PHOTOLAKE_S3_ACCESS_KEY "photolake-$(rand 8)"
env_default PHOTOLAKE_S3_SECRET_KEY "$(rand 40)"
env_default PHOTOLAKE_S3_DATA "$S3_DIR"
env_default PHOTOLAKE_S3_LOGS "$S3_DIR-logs"
env_default PHOTOLAKE_DATA "$DATA_DIR"
env_default PHOTOLAKE_CONFIG_DIR "$CONFIG_DIR"
env_default PHOTOLAKE_IMPORT_DIR "$HOME_DIR/import"
env_default PHOTOLAKE_LISTEN 127.0.0.1
env_default PHOTOLAKE_PORT 7878
env_default PHOTOLAKE_RESTART unless-stopped
env_default PHOTOLAKE_TAILSCALE 0
# GPU support can change between runs (e.g. toolkit installed later), so always record it.
if grep -qE '^PHOTOLAKE_GPU=' .env; then sed -i "s/^PHOTOLAKE_GPU=.*/PHOTOLAKE_GPU=$GPU/" .env; else echo "PHOTOLAKE_GPU=$GPU" >> .env; fi
# Source builds (and `photolake-ctl update` for them) read this checkout.
[ "$FROM_SOURCE" = 1 ] && env_default PHOTOLAKE_SRC "$SRC_DIR"
info "Settings saved to $HOME_DIR/.env (private to you)"

mkdir -p "${XDG_CONFIG_HOME:-$HOME/.config}/photolake"
echo "$HOME_DIR" > "${XDG_CONFIG_HOME:-$HOME/.config}/photolake/home"
mkdir -p "$HOME/.local/bin"
ln -sf "$HOME_DIR/scripts/photolake-ctl" "$HOME/.local/bin/photolake-ctl"
ctl() { PHOTOLAKE_HOME="$HOME_DIR" "$HOME_DIR/scripts/photolake-ctl" "$@"; }
compose() {
  local files=(-f docker-compose.yml); [ "$(env_get PHOTOLAKE_GPU)" = 1 ] && files+=(-f compose.gpu.yml)
  docker compose "${files[@]}" "$@"
}

# ---------------- Images ----------------
bold "4/6  Downloading Photolake"
if [ "$FROM_SOURCE" = 1 ]; then
  [ "$IN_CHECKOUT" = 1 ] || die "--from-source must be run from a source checkout."
  info "Building the app image (10-20 minutes the first time)…"
  compose build photolake </dev/null
  if [ ! -f "$DATA_DIR/models/manifest.json" ]; then
    info "Building the bird-model image (downloads ~3 GB of tools, once)…"
    docker build -f "$SRC_DIR/docker/models.Dockerfile" -t photolake-models:latest "$SRC_DIR"
  fi
else
  compose pull photolake rustfs </dev/null
fi
if [ -f "$DATA_DIR/models/manifest.json" ] && ! docker image inspect "$(env_get PHOTOLAKE_MODELS_IMAGE):latest" >/dev/null 2>&1 && [ "$FROM_SOURCE" = 1 ]; then
  info "Bird models already installed ✓"
else
  ctl models </dev/null
fi

# ---------------- Library + password ----------------
bold "5/6  Library and password"
compose up -d rustfs </dev/null
sleep 3
compose run --rm --no-deps -T photolake init </dev/null >/dev/null
info "Library ready ✓"
if ! grep -q '^admin_password_hash' "$CONFIG_DIR/config.toml" 2>/dev/null; then
  info "Choose a password for Photolake (at least 8 characters). You'll use it to sign in,"
  info "including from your phone if you turn on remote access later."
  while :; do
    read -r -s -p "  Password: " p1; echo
    read -r -s -p "  Repeat:   " p2; echo
    [ "$p1" = "$p2" ] || { warn "They don't match; try again."; continue; }
    [ "${#p1}" -ge 8 ] || { warn "Use at least 8 characters."; continue; }
    printf '%s\n' "$p1" | compose run --rm --no-deps -T photolake passwd >/dev/null && break
  done
  info "Password saved ✓"
fi

# ---------------- Start ----------------
bold "6/6  Starting"
mkdir -p "$HOME/.local/share/applications"
cat > "$HOME/.local/share/applications/photolake.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Photolake
Comment=Your photo library
Exec=xdg-open http://localhost:$(env_get PHOTOLAKE_PORT)
Icon=folder-pictures
Categories=Graphics;Photography;
EOF
if [ "$START" = 1 ]; then
  ctl start </dev/null
  xdg-open "http://localhost:$(env_get PHOTOLAKE_PORT)" >/dev/null 2>&1 || true
fi

bold "All set!"
info "Open Photolake:   http://localhost:$(env_get PHOTOLAKE_PORT)   (also in your app menu)"
info "Plug in a camera card and photos import automatically."
info "Control it with:  photolake-ctl status | stop | start | update | help"
case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) info "(Open a new terminal for the photolake-ctl command to be found.)" ;; esac
