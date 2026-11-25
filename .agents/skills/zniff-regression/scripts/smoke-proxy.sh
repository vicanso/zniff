#!/usr/bin/env bash
# Isolated Layer-2 live proxy & MCP smoke regression for zniff.
# Uses a temporary --data-dir and ephemeral localhost ports so user config,
# CA certificates, and macOS/Linux system proxy settings are never touched.
#
# Usage:
#   .agents/skills/zniff-regression/scripts/smoke-proxy.sh
#   .agents/skills/zniff-regression/scripts/smoke-proxy.sh --with-external

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../../../.." && pwd)"
WITH_EXTERNAL=0
for arg in "$@"; do
  case "$arg" in
    --with-external) WITH_EXTERNAL=1 ;;
    -h|--help)
      sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "Unknown argument: $arg" >&2
      exit 2
      ;;
  esac
done

DATA_DIR="$(mktemp -d "${TMPDIR:-/tmp}/zniff-smoke.XXXXXX")"
ORIGIN_PID=""
ZNIFF_PID=""

cleanup() {
  local rc=$?
  if [ -n "$ZNIFF_PID" ] && kill -0 "$ZNIFF_PID" 2>/dev/null; then
    kill "$ZNIFF_PID" 2>/dev/null || true
    wait "$ZNIFF_PID" 2>/dev/null || true
  fi
  if [ -n "$ORIGIN_PID" ] && kill -0 "$ORIGIN_PID" 2>/dev/null; then
    kill "$ORIGIN_PID" 2>/dev/null || true
    wait "$ORIGIN_PID" 2>/dev/null || true
  fi
  rm -rf "$DATA_DIR"
  exit "$rc"
}
trap cleanup EXIT INT TERM

pick_ports() {
  python3 - <<'PY'
import socket
socks = []
ports = []
for _ in range(3):
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.bind(("127.0.0.1", 0))
    ports.append(s.getsockname()[1])
    socks.append(s)
for s in socks:
    s.close()
print(*ports)
PY
}

read -r ORIGIN_PORT WEB_PORT PROXY_PORT <<<"$(pick_ports)"

echo "==> [1/7] Building zniff debug binary..."
cargo build --manifest-path "$ROOT_DIR/app/Cargo.toml" --quiet

BIN_PATH="${CARGO_TARGET_DIR:-$ROOT_DIR/app/target}/debug/zniff"
if [ ! -x "$BIN_PATH" ]; then
  BIN_PATH="$(cargo metadata --manifest-path "$ROOT_DIR/app/Cargo.toml" --format-version 1 --no-deps \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/debug/zniff"
fi
test -x "$BIN_PATH" || { echo "zniff binary not found at $BIN_PATH" >&2; exit 1; }

echo "==> [2/7] Starting local HTTP origin (127.0.0.1:$ORIGIN_PORT) and isolated zniff --web (web=$WEB_PORT, proxy=$PROXY_PORT)..."
python3 - "$ORIGIN_PORT" >"$DATA_DIR/origin.log" 2>&1 <<'PY' &
import json, sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        sig = self.headers.get("X-Smoke-Sig", "")
        body = json.dumps({"ok": True, "method": "GET", "path": self.path, "sig": sig}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        n = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(n) if n > 0 else b""
        if self.path == "/__hook__":
            req = json.loads(raw.decode())
            if req.get("phase") == "request":
                out = json.dumps({"headers": [["X-Smoke-Hook", "from-hook"]]})
            else:
                out = "{}"
            body = out.encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        body = json.dumps({
            "ok": True,
            "method": "POST",
            "path": self.path,
            "len": len(raw),
            "hook_hdr": self.headers.get("X-Smoke-Hook", ""),
        }).encode()
        self.send_response(201)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, fmt, *args):
        pass

ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
PY
ORIGIN_PID=$!

ORIGIN_READY=0
for _ in $(seq 1 50); do
  if python3 - "$ORIGIN_PORT" <<'PY'
import socket, sys
s = socket.socket()
s.settimeout(0.2)
try:
    s.connect(("127.0.0.1", int(sys.argv[1])))
    sys.exit(0)
except OSError:
    sys.exit(1)
finally:
    s.close()
PY
  then
    ORIGIN_READY=1
    break
  fi
  sleep 0.1
done
if [ "$ORIGIN_READY" -ne 1 ]; then
  echo "Local origin did not start listening on 127.0.0.1:$ORIGIN_PORT" >&2
  cat "$DATA_DIR/origin.log" >&2 || true
  exit 1
fi

ZNIFF_WEB_ROOT="$ROOT_DIR/app/.nsh/web" \
  "$BIN_PATH" --data-dir "$DATA_DIR" --web --web-port "$WEB_PORT" --port "$PROXY_PORT" \
  >"$DATA_DIR/zniff.log" 2>&1 &
ZNIFF_PID=$!

WEB_URL="http://127.0.0.1:$WEB_PORT"
PROXY_URL="http://127.0.0.1:$PROXY_PORT"

for _ in $(seq 1 40); do
  if curl -fsS "$WEB_URL/api/state" >/dev/null 2>&1; then
    break
  fi
  sleep 0.15
done
curl -fsS "$WEB_URL/api/state" >/dev/null

echo "==> [3/7] Starting proxy capture via /api/capture..."
curl -fsS -X POST "$WEB_URL/api/capture" \
  -H "Content-Type: application/json" \
  -d '{"on":true}' >/dev/null

PROXY_READY=0
for _ in $(seq 1 50); do
  if python3 - "$PROXY_PORT" <<'PY'
import socket, sys
s = socket.socket()
s.settimeout(0.2)
try:
    s.connect(("127.0.0.1", int(sys.argv[1])))
    sys.exit(0)
except OSError:
    sys.exit(1)
finally:
    s.close()
PY
  then
    PROXY_READY=1
    break
  fi
  sleep 0.1
done
if [ "$PROXY_READY" -ne 1 ]; then
  echo "Proxy did not start listening on 127.0.0.1:$PROXY_PORT" >&2
  cat "$DATA_DIR/zniff.log" >&2 || true
  exit 1
fi

echo "==> [4/7] Sending HTTP GET/POST traffic through proxy..."
GET_RESP="$(curl -fsS -x "$PROXY_URL" "http://127.0.0.1:$ORIGIN_PORT/v1/ping?from=smoke")"
POST_RESP="$(curl -fsS -x "$PROXY_URL" -X POST "http://127.0.0.1:$ORIGIN_PORT/v1/echo" -H "Content-Type: application/json" -d '{"domain":"api.vicanso.dev"}')"
python3 - "$GET_RESP" "$POST_RESP" <<'PY'
import json, sys
g = json.loads(sys.argv[1])
p = json.loads(sys.argv[2])
assert g["ok"] and g["path"] == "/v1/ping?from=smoke", g
assert p["ok"] and p["method"] == "POST" and p["len"] > 0, p
PY

echo "==> [5/7] Verifying MCP list_flows, get_flow_detail, and replay_flow..."
mcp_call() {
  local id="$1"
  local name="$2"
  local args="$3"
  curl -fsS -X POST "$PROXY_URL/mcp" \
    -H "Content-Type: application/json" \
    -d "{\"jsonrpc\":\"2.0\",\"id\":$id,\"method\":\"tools/call\",\"params\":{\"name\":\"$name\",\"arguments\":$args}}"
}

FLOWS_JSON="$(mcp_call 1 list_flows '{"limit":10}')"
FIRST_ID="$(python3 - "$FLOWS_JSON" <<'PY'
import json, sys
resp = json.loads(sys.argv[1])
text = resp["result"]["content"][0]["text"]
data = json.loads(text)
flows = data["flows"]
assert len(flows) >= 2, f"expected >=2 flows, got {len(flows)}"
print(flows[0]["id"])
PY
)"

DETAIL_JSON="$(mcp_call 2 get_flow_detail "{\"id\":$FIRST_ID}")"
python3 - "$DETAIL_JSON" <<'PY'
import json, sys
resp = json.loads(sys.argv[1])
detail = json.loads(resp["result"]["content"][0]["text"])
assert detail.get("id") is not None and detail.get("url"), detail
PY

REPLAY_JSON="$(mcp_call 3 replay_flow "{\"id\":$FIRST_ID}")"
python3 - "$REPLAY_JSON" <<'PY'
import json, sys
resp = json.loads(sys.argv[1])
assert "error" not in resp, resp
PY

echo "==> [6/7] Verifying MCP set_rules (map-local + break), manage_breaks, HAR/HTML export, and clear_flows..."
MAP_FILE="$DATA_DIR/mock.json"
printf '{"mocked":true,"host":"api.vicanso.dev"}\n' >"$MAP_FILE"
SET_RULES_RESP="$(mcp_call 4 set_rules "{\"rules\":\"map-local 127.0.0.1/mock $MAP_FILE\\nbreak 127.0.0.1/hold\\nheader 127.0.0.1/signed req set X-Smoke-Sig {{hmac_sha256(\\\"k\\\", req.path)}}\\nhook 127.0.0.1/hooked http://127.0.0.1:$ORIGIN_PORT/__hook__ req\",\"rules_enabled\":true,\"breaks_enabled\":true}")"
python3 - "$SET_RULES_RESP" <<'PY'
import json, sys
resp = json.loads(sys.argv[1])
assert not resp.get("result", {}).get("isError"), resp
PY

HOOKED_RESP="$(curl -fsS -x "$PROXY_URL" -X POST "http://127.0.0.1:$ORIGIN_PORT/hooked" -d '{"host":"api.vicanso.dev"}')"
python3 - "$HOOKED_RESP" <<'PY'
import json, sys
data = json.loads(sys.argv[1])
assert data.get("hook_hdr") == "from-hook", data
PY

SIGNED_RESP="$(curl -fsS -x "$PROXY_URL" "http://127.0.0.1:$ORIGIN_PORT/signed")"
python3 - "$SIGNED_RESP" <<'PY'
import hashlib, hmac, json, sys
data = json.loads(sys.argv[1])
expected = hmac.new(b"k", b"/signed", hashlib.sha256).hexdigest()
assert data.get("sig") == expected, f"expected {expected}, got {data}"
PY

MOCK_RESP="$(curl -fsS -x "$PROXY_URL" "http://127.0.0.1:$ORIGIN_PORT/mock")"
python3 - "$MOCK_RESP" <<'PY'
import json, sys
data = json.loads(sys.argv[1])
assert data.get("mocked") is True, data
PY

# Send a request that pauses at the breakpoint, then release it via manage_breaks.
HOLD_OUT="$DATA_DIR/hold.out"
curl -fsS -x "$PROXY_URL" --max-time 10 "http://127.0.0.1:$ORIGIN_PORT/hold" >"$HOLD_OUT" 2>"$DATA_DIR/hold.err" &
HOLD_PID=$!

RELEASED=0
for _ in $(seq 1 30); do
  BRK_JSON="$(mcp_call 5 manage_breaks '{"action":"list"}')"
  if python3 - "$BRK_JSON" <<'PY'
import json, sys
resp = json.loads(sys.argv[1])
data = json.loads(resp["result"]["content"][0]["text"])
sys.exit(0 if data.get("paused_count", 0) > 0 else 1)
PY
  then
    mcp_call 6 manage_breaks '{"action":"continue_all"}' >/dev/null
    RELEASED=1
    break
  fi
  sleep 0.1
done
wait "$HOLD_PID"
test "$RELEASED" -eq 1 || { echo "Breakpoint did not pause as expected" >&2; exit 1; }
python3 - "$(cat "$HOLD_OUT")" <<'PY'
import json, sys
data = json.loads(sys.argv[1])
assert data["ok"] and data["path"] == "/hold", data
PY

HAR_OUT="$(curl -fsS "$WEB_URL/api/export.har")"
python3 - "$HAR_OUT" <<'PY'
import json, sys
har = json.loads(sys.argv[1])
entries = har["log"]["entries"]
assert len(entries) >= 3, f"expected >=3 HAR entries, got {len(entries)}"
PY

HTML_LEN="$(curl -fsS "$WEB_URL/api/export.html" | wc -c | tr -d ' ')"
test "$HTML_LEN" -gt 500 || { echo "HTML export unexpectedly small: $HTML_LEN" >&2; exit 1; }

OPENAPI_OUT="$(curl -fsS "$WEB_URL/api/export.openapi.json")"
python3 - "$OPENAPI_OUT" <<'PY'
import json, sys
spec = json.loads(sys.argv[1])
assert spec.get("openapi") == "3.0.3", spec
paths = spec.get("paths", {})
assert len(paths) >= 3, f"expected >=3 OpenAPI paths, got {len(paths)}"
PY

OPENAPI_YAML="$(curl -fsS "$WEB_URL/api/export.openapi.yaml")"
python3 - "$OPENAPI_YAML" <<'PY'
import sys
text = sys.argv[1]
assert "openapi: 3.0.3" in text and "paths:" in text, text[:300]
PY

MCP_OA="$(mcp_call 9 generate_openapi '{"format":"yaml"}')"
python3 - "$MCP_OA" <<'PY'
import json, sys
resp = json.loads(sys.argv[1])
assert not resp.get("result", {}).get("isError"), resp
data = json.loads(resp["result"]["content"][0]["text"])
assert data.get("operations", 0) >= 3, data
assert data.get("spec", {}).get("openapi") == "3.0.3", data
assert "openapi: 3.0.3" in data.get("yaml", ""), data
PY

if [ "$WITH_EXTERNAL" -eq 1 ]; then
  echo "==> [6b/7] Running external HTTPS/H2/SSE smoke via scripts/test-traffic.sh..."
  ZNIFF_CA_CERT="$DATA_DIR/ca.crt" sh "$ROOT_DIR/scripts/test-traffic.sh" --proxy "$PROXY_URL"
fi

echo "==> [7/7] Verifying MCP clear_flows..."
mcp_call 7 clear_flows '{}' >/dev/null
AFTER_CLEAR="$(mcp_call 8 list_flows '{"limit":10}')"
python3 - "$AFTER_CLEAR" <<'PY'
import json, sys
resp = json.loads(sys.argv[1])
data = json.loads(resp["result"]["content"][0]["text"])
assert len(data["flows"]) == 0, f"expected 0 flows after clear, got {len(data['flows'])}"
PY

echo "PASS: Isolated Layer-2 zniff proxy & MCP smoke regression succeeded."
