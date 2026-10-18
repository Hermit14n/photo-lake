# syntax=docker/dockerfile:1.7
# Photolake app image. Two variants:
#   docker build --target runtime-cpu  -t photolake:cpu  .
#   docker build --target runtime-cuda -t photolake:cuda .   (NVIDIA GPU; needs driver >= 525)
# Bird models ship separately (docker/models.Dockerfile) so app updates stay small.

ARG RUST_IMAGE=rust:1.99-trixie
ARG NODE_IMAGE=node:22-bookworm-slim

# ---------- web UI ----------
FROM ${NODE_IMAGE} AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run build

# ---------- Rust builds ----------
FROM ${RUST_IMAGE} AS rust-base
WORKDIR /src
COPY rust-toolchain.toml ./
RUN rustup show active-toolchain || rustup toolchain install
COPY Cargo.toml Cargo.lock ./
COPY .cargo .cargo
COPY crates crates

FROM rust-base AS build-cpu
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target,id=photolake-target-cpu \
    cargo build --release --locked -p photolake-cli \
 && cp target/release/photolake /photolake

FROM rust-base AS build-cuda
COPY scripts/fetch-onnxruntime-gpu.sh scripts/
RUN scripts/fetch-onnxruntime-gpu.sh /ort
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target,id=photolake-target-cuda \
    cargo build --release --locked -p photolake-cli --features cuda \
 && cp target/release/photolake /photolake

# ---------- runtimes ----------
FROM debian:trixie-slim AS runtime-cpu
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates rclone tzdata \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build-cpu /photolake /opt/photolake/photolake
COPY --from=web /src/web/dist /opt/photolake/web
ENV PHOTOLAKE_CONFIG=/config/config.toml \
    PHOTOLAKE_DATA_DIR=/data \
    PHOTOLAKE_WEB_DIR=/opt/photolake/web \
    PHOTOLAKE_BIND=0.0.0.0:7878 \
    HOME=/data/home \
    RUST_LOG=info,iceberg=warn,datafusion=warn,sqlx=warn,nom_exif=error,rawler=warn,ort=error
EXPOSE 7878
HEALTHCHECK --interval=30s --timeout=5s --start-period=120s CMD ["/opt/photolake/photolake", "health"]
ENTRYPOINT ["/opt/photolake/photolake"]
CMD ["serve"]
LABEL org.opencontainers.image.title="Photolake" \
      org.opencontainers.image.description="Photo lakehouse with bird identification (CPU)" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0"

FROM nvidia/cuda:13.4.2-cudnn-runtime-ubuntu24.04 AS runtime-cuda
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates rclone tzdata \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build-cuda /photolake /opt/photolake/photolake
COPY --from=build-cuda /ort/lib /opt/photolake/lib
COPY --from=web /src/web/dist /opt/photolake/web
ENV PHOTOLAKE_CONFIG=/config/config.toml \
    PHOTOLAKE_DATA_DIR=/data \
    PHOTOLAKE_WEB_DIR=/opt/photolake/web \
    PHOTOLAKE_BIND=0.0.0.0:7878 \
    PHOTOLAKE_ML_DEVICE=auto \
    HOME=/data/home \
    NVIDIA_VISIBLE_DEVICES=all \
    NVIDIA_DRIVER_CAPABILITIES=compute,utility \
    RUST_LOG=info,iceberg=warn,datafusion=warn,sqlx=warn,nom_exif=error,rawler=warn,ort=error
EXPOSE 7878
HEALTHCHECK --interval=30s --timeout=5s --start-period=120s CMD ["/opt/photolake/photolake", "health"]
ENTRYPOINT ["/opt/photolake/photolake"]
CMD ["serve"]
LABEL org.opencontainers.image.title="Photolake" \
      org.opencontainers.image.description="Photo lakehouse with bird identification (NVIDIA GPU)" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0"
