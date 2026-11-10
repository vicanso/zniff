# Operator console on 8080. The capture proxy port stays closed until the
# console starts capture, then it listens on 8877.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY app/Cargo.toml app/Cargo.lock app/
COPY app/src app/src
WORKDIR /src/app
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/app/target/release/zniff /usr/local/bin/zniff
EXPOSE 8080
EXPOSE 8877
ENTRYPOINT ["zniff", "--web", "--web-bind", "0.0.0.0", "--bind", "0.0.0.0", "--port", "8877"]
