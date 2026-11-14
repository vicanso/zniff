# syntax=docker/dockerfile:1
#
# The web server, not the desktop window. `zniff --web` listens on 8080 for
# the operator page and opens 8877 only after Start. Clients must set that
# address as their HTTP proxy. This image does not change the system proxy.
#
# Both the headless server binary (built with `--no-default-features`, without
# GTK/GStreamer) and the shared `nsh 0.5` Wasm UI (`/usr/share/zniff/web`) are
# baked into the image out of the box:
#
#   docker run --rm -p 8080:8080 -p 8877:8877 \
#     -v "$PWD/zniff-data:/data" \
#     vicanso/zniff-web:nightly
#
# `ZNIFF_WEB_ROOT` defaults to `/usr/share/zniff/web` and may still be
# overridden to serve a custom UI directory.
#
# Daily and release images are published by .github/workflows/docker.yml.
# The toolchain matches the GitHub workflows (1.99.1).

FROM rust:1.99.1-trixie AS build

ENV CARGO_NET_RETRY=10 \
    CARGO_HTTP_TIMEOUT=120 \
    CARGO_HTTP_MULTIPLEXING=false

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        clang \
        libclang-dev \
        libgstreamer-plugins-base1.0-dev \
        libgstreamer1.0-dev \
        libgtk-4-dev \
        llvm \
        pkg-config \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add wasm32-unknown-unknown

ARG NSH_VERSION=v0.5.0
RUN set -eux; \
    case "$(uname -m)" in \
      x86_64|amd64) nsh_arch="linux-x86_64" ;; \
      aarch64|arm64) nsh_arch="linux-arm64" ;; \
      *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;; \
    esac; \
    base="https://github.com/longbridge/nsh/releases/download/${NSH_VERSION}"; \
    mkdir -p /tmp/nsh-dl /opt; \
    for asset in "nsh-${nsh_arch}.tar.gz" "nsh-web.tar.gz"; do \
      curl -fsSL --retry 5 "${base}/${asset}" -o "/tmp/nsh-dl/${asset}"; \
      curl -fsSL --retry 5 "${base}/${asset}.sha256" -o "/tmp/nsh-dl/${asset}.sha256"; \
      (cd /tmp/nsh-dl && sha256sum -c "${asset}.sha256"); \
      tar -xzf "/tmp/nsh-dl/${asset}" -C /opt; \
    done; \
    rm -rf /tmp/nsh-dl; \
    /opt/nsh/bin/nsh --version

WORKDIR /src
COPY app/Cargo.toml app/Cargo.lock app/build.rs app/nsh.json app/
COPY app/assets app/assets
COPY app/src app/src
WORKDIR /src/app

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/app/target,sharing=locked \
    cargo build --release --locked --no-default-features \
    && cp target/release/zniff /zniff \
    && /opt/nsh/bin/nsh package --target web -v /src/app -o /web-pkg \
    && mv /web-pkg/zniff /web \
    && test -f /web/index.html \
    && rm -rf /src/app/.nsh /web-pkg

FROM debian:trixie-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 --user-group zniff \
    && mkdir -p /data \
    && chown zniff:zniff /data

COPY --from=build /zniff /usr/local/bin/zniff
COPY --from=build --chown=zniff:zniff /web /usr/share/zniff/web

ENV ZNIFF_DATA_DIR=/data \
    ZNIFF_WEB_ROOT=/usr/share/zniff/web
VOLUME ["/data"]

USER zniff
WORKDIR /home/zniff
EXPOSE 8080
EXPOSE 8877
# Relay port for `zniff --web --relay` (see README).
EXPOSE 8879
ENTRYPOINT ["zniff", "--web", "--web-bind", "0.0.0.0", "--bind", "0.0.0.0", "--port", "8877"]
