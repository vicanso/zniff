#!/usr/bin/env bash
# scripts/demo-video/lib.sh — shared setup for the demo recording.
# Sourced by scripts/make-demo-video.sh (and ad-hoc calibration runs).

DEMO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$DEMO_DIR/../.." && pwd)"
WORK="${WORK:-/tmp/zniff-demo}"
DATA_DIR="$WORK/data"
MOCK_PORT="${MOCK_PORT:-19876}"
PROXY_PORT="${PROXY_PORT:-18878}"
WIN_W=1280
WIN_H=720
export WIN_X=116
export WIN_Y=120
PROXY="http://127.0.0.1:$PROXY_PORT"
MOCK="http://127.0.0.1:$MOCK_PORT"
DRV="$WORK/driver"

mkdir -p "$WORK"

demo_build_driver() {
    if [ ! -x "$DRV" ] || [ "$DEMO_DIR/driver.swift" -nt "$DRV" ]; then
        echo "==> Compiling input driver..."
        swiftc -O "$DEMO_DIR/driver.swift" -o "$DRV"
    fi
}

demo_find_binary() {
    if [ -n "${ZNIFF_BIN:-}" ] && [ -x "$ZNIFF_BIN" ]; then
        return
    fi
    local target
    target="$(cargo metadata --manifest-path "$REPO_ROOT/app/Cargo.toml" --format-version 1 --no-deps 2>/dev/null |
        python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')"
    if [ "${DEMO_SKIP_BUILD:-0}" != "1" ]; then
        echo "==> Building zniff (debug)..."
        cargo build --manifest-path "$REPO_ROOT/app/Cargo.toml" >&2
    fi
    ZNIFF_BIN="$target/debug/zniff"
}

demo_start_mock() {
    local png="$WORK/logo.png"
    sips -s format png -Z 128 "$REPO_ROOT/icons/zniff.icns" --out "$png" >/dev/null 2>&1 || true
    python3 "$DEMO_DIR/mock_server.py" "$MOCK_PORT" "$png" &
    MOCK_PID=$!
    for _ in {1..40}; do
        curl -s -o /dev/null "$MOCK/api/v1/cart" && return
        sleep 0.1
    done
}

demo_write_config() {
    rm -rf "$DATA_DIR"
    mkdir -p "$DATA_DIR"
    cat <<EOF >"$DATA_DIR/config.dev.toml"
listen_port = $PROXY_PORT
capturing = true
window_width = $WIN_W
window_height = $WIN_H
filter = ""
saved_profiles = []
language = "${DEMO_LANG:-en}"
rules = """
break */api/v1/checkout
"""
EOF
}

demo_launch_app() {
    ZNIFF_DEV=1 "$ZNIFF_BIN" --data-dir "$DATA_DIR" &
    ZNIFF_PID=$!
    for _ in {1..60}; do
        "$DRV" bounds "$ZNIFF_PID" >/dev/null 2>&1 && break
        sleep 0.25
    done
    osascript <<OSA >/dev/null 2>&1 || true
tell application "System Events" to tell process "zniff"
    set frontmost to true
    set size of window 1 to {$WIN_W, $WIN_H}
    set position of window 1 to {$WIN_X, $WIN_Y}
end tell
OSA
    sleep 0.8
    WID="$("$DRV" wid "$ZNIFF_PID")"
    echo "==> zniff window $WID at $("$DRV" bounds "$ZNIFF_PID")"
    # Wait until the proxy port accepts connections.
    for _ in {1..40}; do
        nc -z 127.0.0.1 "$PROXY_PORT" 2>/dev/null && return
        sleep 0.15
    done
}

demo_front() {
    osascript -e 'tell application "System Events" to tell process "zniff" to set frontmost to true' >/dev/null 2>&1 || true
}

# One request through the proxy; extra curl args follow the path.
req() {
    local method="$1" url="$2"
    shift 2
    curl -s -o /dev/null -x "$PROXY" -X "$method" "$url" "$@" || true
}

demo_traffic() {
    local gap="${1:-0.18}"
    req GET "$MOCK/static/js/app.9f3c2e.js"; sleep "$gap"
    req GET "$MOCK/static/img/logo.png"; sleep "$gap"
    req POST "$MOCK/api/v1/auth/login" -H 'Content-Type: application/json' \
        -d '{"email":"alice@zniff.app","password":"••••••••"}'; sleep "$gap"
    req GET "$MOCK/api/v1/products?page=1&size=4&sort=popular"; sleep "$gap"
    req POST "$MOCK/api/graphql" -H 'Content-Type: application/json' \
        -d '{"operationName":"ViewerOrders","query":"query ViewerOrders($first: Int!) {\n  viewer {\n    id\n    name\n    email\n    plan\n    orders(first: $first) {\n      totalCount\n      edges { node { id status total } }\n    }\n  }\n}","variables":{"first":3}}'; sleep "$gap"
    req GET "$MOCK/api/v1/cart" -H "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.e30.x"; sleep "$gap"
    req GET "$MOCK/api/v1/feature-flags" -H 'If-None-Match: "ff-20261114"'; sleep "$gap"
    req GET "$MOCK/api/v1/telemetry" -H 'Accept: application/x-protobuf'; sleep "$gap"
    req GET "$MOCK/api/v1/recommendations"; sleep "$gap"
    req POST "$MOCK/api/v1/orders" -H 'Content-Type: application/json' \
        -d '{"items":[{"sku":"sku_1001","qty":1},{"sku":"sku_1003","qty":2}],"coupon":"ZNIFF10"}'; sleep "$gap"
    req GET "$MOCK/api/v1/billing/invoices"; sleep "$gap"
    req GET "http://127.0.0.1:19999/api/v1/inventory"
}

demo_cleanup() {
    for pid in ${BP_PIDS:-} ${ZNIFF_PID:-} ${MOCK_PID:-}; do
        kill "$pid" 2>/dev/null || true
    done
    if [ -n "${REC_PID:-}" ] && kill -0 "$REC_PID" 2>/dev/null; then
        kill -INT "$REC_PID" 2>/dev/null || true
        wait "$REC_PID" 2>/dev/null || true
    fi
    if [ -n "${ORIGINAL_DARK:-}" ]; then
        osascript -e "tell application \"System Events\" to tell appearance preferences to set dark mode to $ORIGINAL_DARK" >/dev/null 2>&1 || true
    fi
}
