# syntax=docker/dockerfile:1
#
# The web server, not the desktop window. `zniff --web` listens on 8080 for
# the operator page and opens 8877 only after Start. Clients must set that
# address as their HTTP proxy. This image does not change the system proxy.
#
# The page baked into the image is the small console. To serve the shared
# UI and persist the generated root CA (`ca.crt` / `ca.key`) and `config.toml`
# across container restarts, mount `/web` and `/data`:
#
#   docker run --rm -p 8080:8080 -p 8877:8877 \
#     -e ZNIFF_WEB_ROOT=/web \
#     -v "$PWD/app/.nsh/web:/web:ro" \
#     -v "$PWD/zniff-data:/data" \
#     vicanso/zniff-web:nightly
#
# Daily and release images are published by .github/workflows/docker.yml.
# Built with `--no-default-features` so `nsh-platform-linux` (GTK / GStreamer)
# is not linked into the headless server binary. The toolchain matches the
# GitHub workflows (1.99.1).

FROM rust:1.99.1-trixie AS build

ENV CARGO_NET_RETRY=10 \
    CARGO_HTTP_TIMEOUT=120 \
    CARGO_HTTP_MULTIPLEXING=false

WORKDIR /src
COPY app/Cargo.toml app/Cargo.lock app/nsh.json app/
COPY app/src app/src
WORKDIR /src/app

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/app/target,sharing=locked \
    cargo build --release --locked --no-default-features \
    && cp target/release/zniff /zniff

FROM debian:trixie-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 --user-group zniff \
    && mkdir -p /data \
    && chown zniff:zniff /data

COPY --from=build /zniff /usr/local/bin/zniff

ENV ZNIFF_DATA_DIR=/data
VOLUME ["/data"]

USER zniff
WORKDIR /home/zniff
EXPOSE 8080
EXPOSE 8877
# Relay port for `zniff --web --relay` (see README).
EXPOSE 8879
ENTRYPOINT ["zniff", "--web", "--web-bind", "0.0.0.0", "--bind", "0.0.0.0", "--port", "8877"]
