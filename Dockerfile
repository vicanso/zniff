# syntax=docker/dockerfile:1
#
# The web server, not the desktop window. `zniff --web` listens on 8080 for
# the operator page and opens 8877 only after Start. Clients must set that
# address as their HTTP proxy. This image does not change the system proxy.
#
# The page baked into the image is the small console. To serve the shared
# UI, mount a directory from `make web-ui`:
#
#   docker run --rm -p 8080:8080 -p 8877:8877 \
#     -e ZNIFF_WEB_ROOT=/web \
#     -v "$PWD/app/.nsh/web:/web:ro" \
#     vicanso/zniff-web:nightly
#
# Daily and release images are published by .github/workflows/docker.yml.
# The binary is the desktop build, so it links GTK even though --web never
# opens a window. The toolchain matches the GitHub workflows (1.99.1).

FROM rust:1.99.1-bookworm AS build

RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libgtk-4-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

ENV CARGO_NET_RETRY=10 \
    CARGO_HTTP_TIMEOUT=120 \
    CARGO_HTTP_MULTIPLEXING=false

WORKDIR /src
COPY app/Cargo.toml app/Cargo.lock app/nsh.json app/
COPY app/src app/src
WORKDIR /src/app

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/app/target,sharing=locked \
    cargo build --release --locked \
    && cp target/release/zniff /zniff

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libgtk-4-1 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 --user-group zniff

COPY --from=build /zniff /usr/local/bin/zniff

USER zniff
WORKDIR /home/zniff
EXPOSE 8080
EXPOSE 8877
ENTRYPOINT ["zniff", "--web", "--web-bind", "0.0.0.0", "--bind", "0.0.0.0", "--port", "8877"]
