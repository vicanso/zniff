#!/usr/bin/env bash
# End-to-End functional test for zniff v0.4.x & v0.5 backend/proxy/web/MCP features.
# Runs against a real compiled `zniff --web` process + live capture proxy + local mock HTTP upstream.
# Fully isolated: uses a temp --data-dir, ephemeral localhost ports, and never touches
# user config.toml, ca.crt, or system proxy settings.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Scrub ambient proxy/zniff env vars so nothing interferes with isolation.
unset HTTP_PROXY HTTPS_PROXY ALL_PROXY NO_PROXY http_proxy https_proxy all_proxy no_proxy
unset ZNIFF_DEV ZNIFF_ALLOW_HOSTS ZNIFF_PROXY_AUTH ZNIFF_WEB_AUTH ZNIFF_WEB_PREFIX ZNIFF_WEB_ROOT

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/zniff-v05-e2e.XXXXXX")"
DATA_DIR="$TMP_DIR/data"
mkdir -p "$DATA_DIR"

ZNIFF_PID=""
UPSTREAM_PID=""
BG_PIDS=()

cleanup() {
  local rc=$?
  for pid in "${BG_PIDS[@]:-}"; do
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
  if [ -n "$ZNIFF_PID" ] && kill -0 "$ZNIFF_PID" 2>/dev/null; then
    kill "$ZNIFF_PID" 2>/dev/null || true
    wait "$ZNIFF_PID" 2>/dev/null || true
  fi
  if [ -n "$UPSTREAM_PID" ] && kill -0 "$UPSTREAM_PID" 2>/dev/null; then
    kill "$UPSTREAM_PID" 2>/dev/null || true
    wait "$UPSTREAM_PID" 2>/dev/null || true
  fi
  if [ "$rc" -ne 0 ] && [ -f "$TMP_DIR/zniff.log" ]; then
    echo "=== zniff log (tail 60) ===" >&2
    tail -n 60 "$TMP_DIR/zniff.log" >&2 || true
  fi
  rm -rf "$TMP_DIR"
  exit "$rc"
}
trap cleanup EXIT INT TERM

PASS_COUNT=0
FAIL_COUNT=0
pass() {
  PASS_COUNT=$((PASS_COUNT + 1))
  printf '  [PASS] %s\n' "$1"
}
fail() {
  FAIL_COUNT=$((FAIL_COUNT + 1))
  printf '  [FAIL] %s\n' "$1" >&2
  exit 1
}

# Resolve zniff binary.
if [ -z "${ZNIFF_BIN:-}" ]; then
  TARGET_DIR="$(cargo metadata --manifest-path "$REPO_ROOT/app/Cargo.toml" --format-version 1 --no-deps \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
  ZNIFF_BIN="$TARGET_DIR/debug/zniff"
fi
if [ ! -x "$ZNIFF_BIN" ]; then
  echo "==> Building zniff binary..."
  cargo build --manifest-path "$REPO_ROOT/app/Cargo.toml"
fi

# Ensure Wasm web UI bundle exists.
WEB_ROOT="$REPO_ROOT/app/.nsh/web"
if [ ! -f "$WEB_ROOT/index.html" ]; then
  echo "==> Building Wasm web UI (make web-ui)..."
  make -C "$REPO_ROOT" web-ui
fi

# Allocate 3 distinct ephemeral localhost ports.
read -r WEB_PORT PROXY_PORT UPSTREAM_PORT < <(python3 -c '
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
print(ports[0], ports[1], ports[2])
')

WEB_BASE="http://127.0.0.1:$WEB_PORT"
PROXY_URL="http://127.0.0.1:$PROXY_PORT"
MCP_URL="http://127.0.0.1:$PROXY_PORT/mcp"

echo "==> Starting local mock HTTP upstream on 127.0.0.1:$UPSTREAM_PORT"
cat > "$TMP_DIR/upstream.py" <<'PYEOF'
import http.server
import json
import sys

port = int(sys.argv[1])

class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        pass

    def _handle(self):
        length = int(self.headers.get("Content-Length", "0") or "0")
        body = self.rfile.read(length).decode("utf-8", errors="replace") if length > 0 else ""
        status = 200
        if self.path.startswith("/fail-trigger"):
            payload = {"ok": False, "reason": "fail_flag_detected", "path": self.path}
        elif self.path.startswith("/error-500"):
            status = 500
            payload = {"error": "internal_boom", "path": self.path}
        elif self.path.startswith("/diff-a"):
            payload = {"version": 1, "item": "alpha", "count": 10}
        elif self.path.startswith("/diff-b"):
            payload = {"version": 2, "item": "beta", "count": 20}
        elif self.path.startswith("/mock-target"):
            payload = {"source": "live-upstream", "ok": True}
        else:
            payload = {
                "upstream": True,
                "method": self.command,
                "path": self.path,
                "headers": {k.lower(): v for k, v in self.headers.items()},
                "body": body,
            }
        raw = json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self): self._handle()
    def do_POST(self): self._handle()
    def do_PUT(self): self._handle()
    def do_DELETE(self): self._handle()

server = http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler)
server.serve_forever()
PYEOF
python3 "$TMP_DIR/upstream.py" "$UPSTREAM_PORT" &
UPSTREAM_PID=$!

# Wait for upstream readiness.
for _ in $(seq 1 40); do
  if curl -sS -o /dev/null "http://127.0.0.1:$UPSTREAM_PORT/health" 2>/dev/null; then
    break
  fi
  sleep 0.1
done

echo "==> Starting zniff --web (web=127.0.0.1:$WEB_PORT, proxy=127.0.0.1:$PROXY_PORT, data=$DATA_DIR)"
ZNIFF_WEB_ROOT="$WEB_ROOT" "$ZNIFF_BIN" --web \
  --web-bind 127.0.0.1 --web-port "$WEB_PORT" \
  --bind 127.0.0.1 --port "$PROXY_PORT" \
  --data-dir "$DATA_DIR" \
  >"$TMP_DIR/zniff.log" 2>&1 &
ZNIFF_PID=$!

# Wait for web console readiness.
READY=0
for _ in $(seq 1 60); do
  if curl -sS -o /dev/null "$WEB_BASE/api/state" 2>/dev/null; then
    READY=1
    break
  fi
  sleep 0.1
done
[ "$READY" -eq 1 ] || fail "zniff --web failed to start on $WEB_BASE"

# Start live capture via /api/capture.
CAP_RES="$(curl -sS -X POST "$WEB_BASE/api/capture" -H "Content-Type: application/json" -d '{"on":true}')"
python3 -c 'import json,sys; d=json.loads(sys.argv[1]); assert d.get("capturing") is True, d' "$CAP_RES" \
  || fail "failed to enable capture: $CAP_RES"
[ -f "$DATA_DIR/ca.crt" ] || fail "isolated temp ca.crt not found in $DATA_DIR"

hub_post() {
  curl -sS -X POST "$WEB_BASE/api/hub" -H "Content-Type: application/json" -d "$1"
}

mcp_call() {
  curl -sS -X POST "$MCP_URL" -H "Content-Type: application/json" -d "$1"
}

mcp_tool() {
  local name="$1"
  local args_json="$2"
  local payload
  payload="$(python3 -c 'import json,sys; print(json.dumps({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":sys.argv[1],"arguments":json.loads(sys.argv[2])}}))' "$name" "$args_json")"
  mcp_call "$payload"
}

# ==============================================================================
# 10. Wasm Web Console (run early to confirm serving app/.nsh/web)
# ==============================================================================
echo "==> [10] Wasm Web Console"
INDEX_CODE="$(curl -sS -o "$TMP_DIR/index.html" -w '%{http_code}' "$WEB_BASE/")"
[ "$INDEX_CODE" = "200" ] || fail "GET / returned $INDEX_CODE"
python3 - "$TMP_DIR/index.html" "$WEB_ROOT" "$WEB_BASE" <<'PYEOF'
import os, sys, urllib.request

html_path, web_root, web_base = sys.argv[1], sys.argv[2], sys.argv[3]
html = open(html_path, "r", encoding="utf-8").read()
assert len(html) > 100, "index.html unexpectedly short"

js_files = [f for f in os.listdir(web_root) if f.endswith(".js")]
wasm_files = [f for f in os.listdir(web_root) if f.endswith(".wasm")]
assert js_files, f"no .js files in {web_root}"
assert wasm_files, f"no .wasm files in {web_root}"

for asset in (js_files[0], wasm_files[0]):
    req = urllib.request.urlopen(f"{web_base}/{asset}", timeout=5)
    assert req.status == 200, f"GET /{asset} -> {req.status}"
    data = req.read()
    assert len(data) > 0, f"empty asset /{asset}"
    if asset.endswith(".wasm"):
        ct = req.headers.get("Content-Type", "")
        assert "wasm" in ct.lower(), f"unexpected wasm content-type: {ct}"

for ep in ("/api/state", "/api/hub"):
    req = urllib.request.urlopen(f"{web_base}{ep}", timeout=5)
    assert req.status == 200, f"GET {ep} -> {req.status}"
    assert req.read().startswith(b"{"), f"non-JSON response from {ep}"
PYEOF
pass "Wasm web console serves /, JS, Wasm, /api/state, /api/hub with HTTP 200"

# ==============================================================================
# 1. B1 + B2: Multi-rule matching (workspace + rule group) -> rule_hits / HAR / MCP
# ==============================================================================
echo "==> [1] B1 + B2: Multi-rule matching (workspace + rule group)"
WS_RULES="$(printf 'resolve api.vicanso.dev 127.0.0.1:%s\nheader api.vicanso.dev/b1b2 res set X-Workspace-Rule yes' "$UPSTREAM_PORT")"
GROUPS_JSON='[{"name":"grp-qa","enabled":true,"rules":"header api.vicanso.dev/b1b2 res set X-Group-Rule yes\nquery api.vicanso.dev/b1b2 set traced 1"}]'

hub_post "$(python3 -c 'import json,sys; print(json.dumps({"op":"set_rules","text":sys.argv[1]}))' "$WS_RULES")" >/dev/null
hub_post "$(python3 -c 'import json,sys; print(json.dumps({"op":"set_rule_groups","groups":json.loads(sys.argv[1])}))' "$GROUPS_JSON")" >/dev/null

curl -sS -D "$TMP_DIR/b1b2.hdr" -o "$TMP_DIR/b1b2.body" -x "$PROXY_URL" "http://api.vicanso.dev/b1b2"

grep -qi '^X-Workspace-Rule: yes' "$TMP_DIR/b1b2.hdr" || fail "missing X-Workspace-Rule header"
grep -qi '^X-Group-Rule: yes' "$TMP_DIR/b1b2.hdr" || fail "missing X-Group-Rule header"
grep -q 'traced=1' "$TMP_DIR/b1b2.body" || fail "query rule not applied in upstream request"

FLOWS_JSON="$(curl -sS "$WEB_BASE/api/flows")"
HAR_JSON="$(curl -sS "$WEB_BASE/api/export.har")"

B1_FLOW_ID="$(python3 - "$FLOWS_JSON" "$HAR_JSON" <<'PYEOF'
import json, sys

flows = json.loads(sys.argv[1])["flows"]
target = [f for f in flows if f.get("host") == "api.vicanso.dev" and f.get("path", "").startswith("/b1b2")]
assert target, f"b1b2 flow not found in /api/flows: {flows}"
flow = target[-1]
hits = flow.get("rule_hits", [])
assert len(hits) >= 3, f"expected >=3 rule_hits, got {hits}"
assert any(h.get("group") is None and h.get("verb") == "header" and h.get("line") == 2 for h in hits), f"missing workspace header hit: {hits}"
assert any(h.get("group") == "grp-qa" and h.get("verb") == "header" and h.get("line") == 1 for h in hits), f"missing group header hit: {hits}"
assert any(h.get("group") == "grp-qa" and h.get("verb") == "query" and h.get("line") == 2 for h in hits), f"missing group query hit: {hits}"

har = json.loads(sys.argv[2])
entries = har["log"]["entries"]
har_target = [e for e in entries if "api.vicanso.dev/b1b2" in e["request"]["url"]]
assert har_target, "b1b2 entry not found in HAR"
har_rules = har_target[-1].get("_zniffRules", [])
assert any(h.get("group") is None and h.get("verb") == "header" for h in har_rules), f"missing workspace hit in HAR _zniffRules: {har_rules}"
assert any(h.get("group") == "grp-qa" and h.get("verb") == "query" for h in har_rules), f"missing group hit in HAR _zniffRules: {har_rules}"
print(flow["id"])
PYEOF
)"

MCP_DETAIL="$(mcp_tool "get_flow_detail" "{\"id\":$B1_FLOW_ID}")"
python3 - "$MCP_DETAIL" <<'PYEOF'
import json, sys
rpc = json.loads(sys.argv[1])
assert "result" in rpc and not rpc["result"].get("isError"), rpc
detail = json.loads(rpc["result"]["content"][0]["text"])
hits = detail.get("rule_hits", [])
assert any(h.get("group") is None and h.get("verb") == "header" and h.get("line") == 2 for h in hits), hits
assert any(h.get("group") == "grp-qa" and h.get("verb") == "header" and h.get("line") == 1 for h in hits), hits
assert any(h.get("group") == "grp-qa" and h.get("verb") == "query" and h.get("line") == 2 for h in hits), hits
PYEOF
pass "B1 + B2: multi-rule rule_hits verified in /api/flows, HAR _zniffRules, and MCP get_flow_detail"

# ==============================================================================
# 2. E8: Conditional breakpoints (header=, body=, break-res body=)
# ==============================================================================
echo "==> [2] E8: Conditional breakpoints"
hub_post '{"op":"set_rule_groups","groups":[]}' >/dev/null
E8_RULES="$(cat <<EOF
resolve api.vicanso.dev 127.0.0.1:$UPSTREAM_PORT
break api.vicanso.dev/cond-hdr header=X-Debug:1
break api.vicanso.dev/cond-body body=secret_token
break-res api.vicanso.dev/fail-trigger body=fail_flag
EOF
)"
hub_post "$(python3 -c 'import json,sys; print(json.dumps({"op":"set_rules","text":sys.argv[1]}))' "$E8_RULES")" >/dev/null
hub_post '{"op":"set_breaks_enabled","on":true}' >/dev/null

# Non-matching requests must pass immediately without pausing.
CODE_NM1="$(curl -sS --max-time 3 -o /dev/null -w '%{http_code}' -x "$PROXY_URL" "http://api.vicanso.dev/cond-hdr")"
[ "$CODE_NM1" = "200" ] || fail "non-matching header break request failed: $CODE_NM1"

CODE_NM2="$(curl -sS --max-time 3 -o /dev/null -w '%{http_code}' -x "$PROXY_URL" -X POST -d '{"msg":"harmless"}' "http://api.vicanso.dev/cond-body")"
[ "$CODE_NM2" = "200" ] || fail "non-matching body break request failed: $CODE_NM2"

CODE_NM3="$(curl -sS --max-time 3 -o /dev/null -w '%{http_code}' -x "$PROXY_URL" "http://api.vicanso.dev/ok-res")"
[ "$CODE_NM3" = "200" ] || fail "non-matching response break request failed: $CODE_NM3"

# Matching header break -> pauses -> continue via MCP.
curl -sS --max-time 10 -o "$TMP_DIR/e8_hdr.body" -w '%{http_code}' -x "$PROXY_URL" \
  -H "X-Debug: 1" "http://api.vicanso.dev/cond-hdr" >"$TMP_DIR/e8_hdr.code" &
P1=$!
BG_PIDS+=("$P1")

PAUSED=0
for _ in $(seq 1 40); do
  BL="$(mcp_tool "manage_breaks" '{"action":"list"}')"
  if python3 -c 'import json,sys; d=json.loads(json.loads(sys.argv[1])["result"]["content"][0]["text"]); assert d["paused_count"] >= 1 and d["queue"][0]["side"] == "request"' "$BL" 2>/dev/null; then
    PAUSED=1
    break
  fi
  sleep 0.1
done
[ "$PAUSED" -eq 1 ] || fail "header conditional breakpoint did not pause (last: $BL)"

CONT_RES="$(mcp_tool "manage_breaks" '{"action":"continue"}')"
python3 -c 'import json,sys; r=json.loads(sys.argv[1]); assert not r["result"].get("isError"), r' "$CONT_RES" || fail "manage_breaks continue failed"
wait "$P1"
[ "$(cat "$TMP_DIR/e8_hdr.code")" = "200" ] || fail "resumed header-break request did not return 200"

# Matching request body break + matching response body break -> both pause -> continue_all via MCP.
curl -sS --max-time 10 -o "$TMP_DIR/e8_body.body" -w '%{http_code}' -x "$PROXY_URL" \
  -X POST -H "Content-Type: application/json" -d '{"token":"secret_token"}' \
  "http://api.vicanso.dev/cond-body" >"$TMP_DIR/e8_body.code" &
P2=$!
BG_PIDS+=("$P2")

curl -sS --max-time 10 -o "$TMP_DIR/e8_res.body" -w '%{http_code}' -x "$PROXY_URL" \
  "http://api.vicanso.dev/fail-trigger" >"$TMP_DIR/e8_res.code" &
P3=$!
BG_PIDS+=("$P3")

PAUSED_BOTH=0
for _ in $(seq 1 50); do
  BL="$(mcp_tool "manage_breaks" '{"action":"list"}')"
  if python3 -c '
import json, sys
d = json.loads(json.loads(sys.argv[1])["result"]["content"][0]["text"])
sides = {p["side"] for p in d.get("queue", [])}
assert d["paused_count"] >= 2 and "request" in sides and "response" in sides
' "$BL" 2>/dev/null; then
    PAUSED_BOTH=1
    break
  fi
  sleep 0.1
done
[ "$PAUSED_BOTH" -eq 1 ] || fail "body + break-res conditional breakpoints did not both pause (last: $BL)"

CONT_ALL="$(mcp_tool "manage_breaks" '{"action":"continue_all"}')"
python3 -c 'import json,sys; r=json.loads(sys.argv[1]); assert not r["result"].get("isError"), r' "$CONT_ALL" || fail "manage_breaks continue_all failed"
wait "$P2"
wait "$P3"
[ "$(cat "$TMP_DIR/e8_body.code")" = "200" ] || fail "resumed body-break request failed"
[ "$(cat "$TMP_DIR/e8_res.code")" = "200" ] || fail "resumed break-res request failed"
grep -q 'fail_flag_detected' "$TMP_DIR/e8_res.body" || fail "resumed break-res body mismatch"
pass "E8: conditional breakpoints (header=, body=, break-res body=) pass non-matches, pause matches, and resume via MCP"

# ==============================================================================
# 3. E1 + F4: MCP mock_flow -> writes <data_dir>/mocks/ + live map-local response
# ==============================================================================
echo "==> [3] E1 + F4: MCP mock_flow"
hub_post "$(python3 -c 'import json,sys; print(json.dumps({"op":"set_rules","text":f"resolve api.vicanso.dev 127.0.0.1:{sys.argv[1]}"}))' "$UPSTREAM_PORT")" >/dev/null

LIVE_BODY="$(curl -sS -x "$PROXY_URL" "http://api.vicanso.dev/mock-target")"
grep -q 'live-upstream' <<<"$LIVE_BODY" || fail "unexpected live upstream body: $LIVE_BODY"

MOCK_SRC_ID="$(curl -sS "$WEB_BASE/api/flows" | python3 -c '
import json, sys
flows = json.load(sys.stdin)["flows"]
target = [f for f in flows if f.get("host") == "api.vicanso.dev" and f.get("path") == "/mock-target"]
assert target, "mock-target flow not captured"
print(target[-1]["id"])
')"

MOCK_ARGS="$(python3 -c 'import json,sys; print(json.dumps({"flow_id":int(sys.argv[1]),"rule_type":"map-local","status":201,"body":"{\"source\":\"zniff-mocked\",\"ok\":true}","headers":{"X-Mocked-By":"zniff-e2e"}}))' "$MOCK_SRC_ID")"
MOCK_RES="$(mcp_tool "mock_flow" "$MOCK_ARGS")"
MOCK_FILE="$(python3 - "$MOCK_RES" "$DATA_DIR" <<'PYEOF'
import json, os, sys
rpc = json.loads(sys.argv[1])
assert not rpc["result"].get("isError"), rpc
data = json.loads(rpc["result"]["content"][0]["text"])
mf = data.get("mock_file", "")
assert mf and os.path.isfile(mf), f"mock_file missing on disk: {data}"
mocks_dir = os.path.realpath(os.path.join(sys.argv[2], "mocks"))
assert os.path.realpath(mf).startswith(mocks_dir + os.sep), f"mock_file not under {mocks_dir}: {mf}"
print(mf)
PYEOF
)"

MOCK_CODE="$(curl -sS -D "$TMP_DIR/mock.hdr" -o "$TMP_DIR/mock.body" -w '%{http_code}' -x "$PROXY_URL" "http://api.vicanso.dev/mock-target")"
[ "$MOCK_CODE" = "201" ] || fail "mocked response status expected 201, got $MOCK_CODE"
grep -qi '^X-Mocked-By: zniff-e2e' "$TMP_DIR/mock.hdr" || fail "missing X-Mocked-By header"
grep -q 'zniff-mocked' "$TMP_DIR/mock.body" || fail "mocked body mismatch: $(cat "$TMP_DIR/mock.body")"
pass "E1 + F4: mock_flow wrote $MOCK_FILE and served mocked 201 response via map-local"

# ==============================================================================
# 4. E5: Session markers (/api/hub add_marker, mark: filter, HAR + HTML export)
# ==============================================================================
echo "==> [4] E5: Session markers"
hub_post '{"op":"add_marker","text":"checkout-phase"}' >/dev/null
curl -sS -o /dev/null -x "$PROXY_URL" "http://api.vicanso.dev/after-marker"

ALL_FLOWS="$(curl -sS "$WEB_BASE/api/flows")"
MARK_FLOWS="$(curl -sS "$WEB_BASE/api/flows?filter=mark:checkout-phase")"
MARK_HAR="$(curl -sS "$WEB_BASE/api/export.har")"
MARK_HTML="$(curl -sS "$WEB_BASE/api/export.html")"

python3 - "$ALL_FLOWS" "$MARK_FLOWS" "$MARK_HAR" "$MARK_HTML" <<'PYEOF'
import json, sys

all_flows = json.loads(sys.argv[1])["flows"]
markers = [f for f in all_flows if f.get("protocol") in ("标记", "Marker") and f.get("path") == "checkout-phase"]
assert markers, f"marker flow not found in /api/flows: {all_flows[-5:]}"

mark_flows = json.loads(sys.argv[2])["flows"]
assert any(f.get("protocol") in ("标记", "Marker") and f.get("path") == "checkout-phase" for f in mark_flows), mark_flows
assert any(f.get("path") == "/after-marker" for f in mark_flows), f"section flow missing from mark: filter: {mark_flows}"
assert not any(f.get("path", "").startswith("/b1b2") for f in mark_flows), f"pre-marker flow leaked into mark: filter: {mark_flows}"

har = json.loads(sys.argv[3])
har_markers = har["log"].get("_zniffMarkers", [])
assert any(m.get("label") == "checkout-phase" for m in har_markers), f"missing _zniffMarkers: {har_markers}"
assert any(e.get("_zniffMarker") is True and e.get("_zniffMarkerLabel") == "checkout-phase" for e in har["log"]["entries"]), "missing _zniffMarker entry in HAR"

html = sys.argv[4]
assert "checkout-phase" in html, "marker label missing from HTML export"
PYEOF
pass "E5: session marker verified in /api/flows, mark: filter, HAR, and HTML export"

# ==============================================================================
# 5. D5: Clear with undo (/api/hub clear_undoable + undo_clear)
# ==============================================================================
echo "==> [5] D5: Clear with undo"
BEFORE_CLEAR_COUNT="$(curl -sS "$WEB_BASE/api/flows" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["flows"]))')"
[ "$BEFORE_CLEAR_COUNT" -gt 0 ] || fail "expected >0 flows before clear"

hub_post '{"op":"clear_undoable"}' >/dev/null
AFTER_CLEAR_COUNT="$(curl -sS "$WEB_BASE/api/flows" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["flows"]))')"
[ "$AFTER_CLEAR_COUNT" = "0" ] || fail "expected 0 flows after clear_undoable, got $AFTER_CLEAR_COUNT"

hub_post '{"op":"undo_clear"}' >/dev/null
AFTER_UNDO_COUNT="$(curl -sS "$WEB_BASE/api/flows" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["flows"]))')"
[ "$AFTER_UNDO_COUNT" = "$BEFORE_CLEAR_COUNT" ] || fail "expected $BEFORE_CLEAR_COUNT flows after undo_clear, got $AFTER_UNDO_COUNT"
pass "D5: clear_undoable emptied session and undo_clear restored all $BEFORE_CLEAR_COUNT flows"

# ==============================================================================
# 6. E3: Network preset rules (delay + block * drop)
# ==============================================================================
echo "==> [6] E3: Network preset rules (delay + block drop)"
E3_RULES="$(cat <<EOF
resolve api.vicanso.dev 127.0.0.1:$UPSTREAM_PORT
delay api.vicanso.dev/slow-rule 350ms
block api.vicanso.dev/drop-rule drop
EOF
)"
hub_post "$(python3 -c 'import json,sys; print(json.dumps({"op":"set_rules","text":sys.argv[1]}))' "$E3_RULES")" >/dev/null

DELAY_MS="$(python3 - "$PROXY_URL" <<'PYEOF'
import sys, time, urllib.request
proxy_handler = urllib.request.ProxyHandler({"http": sys.argv[1]})
opener = urllib.request.build_opener(proxy_handler)
t0 = time.monotonic()
resp = opener.open("http://api.vicanso.dev/slow-rule", timeout=5)
assert resp.status == 200, resp.status
resp.read()
elapsed_ms = int((time.monotonic() - t0) * 1000)
assert elapsed_ms >= 300, f"delay rule did not delay enough: {elapsed_ms}ms"
print(elapsed_ms)
PYEOF
)"

DROP_RC=0
curl -sS --max-time 3 -o "$TMP_DIR/drop.out" -w '%{http_code}' -x "$PROXY_URL" "http://api.vicanso.dev/drop-rule" >"$TMP_DIR/drop.code" 2>"$TMP_DIR/drop.err" || DROP_RC=$?
DROP_CODE="$(cat "$TMP_DIR/drop.code" 2>/dev/null || echo "")"
if [ "$DROP_RC" -eq 0 ] && [ "$DROP_CODE" = "200" ]; then
  fail "block drop rule unexpectedly returned HTTP 200"
fi
pass "E3: delay rule delayed ${DELAY_MS}ms and block drop terminated connection (rc=$DROP_RC, code=$DROP_CODE)"

# ==============================================================================
# 7. G2 + G3: Mobile /zniff Android config + iOS HTTPS zniff.local/probe
# ==============================================================================
echo "==> [7] G2 + G3: Mobile /zniff & HTTPS zniff.local/probe"
INSTALL_CODE="$(curl -sS -o "$TMP_DIR/zniff.html" -w '%{http_code}' "http://127.0.0.1:$PROXY_PORT/zniff")"
[ "$INSTALL_CODE" = "200" ] || fail "GET /zniff returned $INSTALL_CODE"
grep -q 'network_security_config.xml' "$TMP_DIR/zniff.html" || fail "missing network_security_config.xml in /zniff"
grep -q 'android:networkSecurityConfig' "$TMP_DIR/zniff.html" || fail "missing android:networkSecurityConfig in /zniff"

FLOWS_BEFORE_PROBE="$(curl -sS "$WEB_BASE/api/flows" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["flows"]))')"
PROBE_CODE="$(curl -sS --cacert "$DATA_DIR/ca.crt" -x "$PROXY_URL" -o "$TMP_DIR/probe.json" -w '%{http_code}' "https://zniff.local/probe")"
[ "$PROBE_CODE" = "200" ] || fail "HTTPS https://zniff.local/probe returned $PROBE_CODE"
python3 - "$TMP_DIR/probe.json" <<'PYEOF'
import json, sys
d = json.load(open(sys.argv[1]))
assert d.get("ok") is True and d.get("host") == "zniff.local" and d.get("tls") == "trusted", d
PYEOF
FLOWS_AFTER_PROBE="$(curl -sS "$WEB_BASE/api/flows" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["flows"]))')"
[ "$FLOWS_BEFORE_PROBE" = "$FLOWS_AFTER_PROBE" ] || fail "zniff.local/probe polluted /api/flows ($FLOWS_BEFORE_PROBE -> $FLOWS_AFTER_PROBE)"
pass "G2 + G3: /zniff contains Android network_security_config.xml; HTTPS zniff.local/probe returned 200 OK JSON with zero flow pollution"

# ==============================================================================
# 8. allow_hosts denied stats + reset_allow_denied
# ==============================================================================
echo "==> [8] allow_hosts denied stats"
hub_post '{"op":"set_allow_hosts","text":"api.vicanso.dev"}' >/dev/null
DENY_CODE="$(curl -sS -o /dev/null -w '%{http_code}' -x "$PROXY_URL" "http://denied.vicanso.dev/secret")"
[ "$DENY_CODE" = "403" ] || fail "expected 403 for denied host, got $DENY_CODE"

HUB_AFTER_DENY="$(curl -sS "$WEB_BASE/api/hub")"
python3 - "$HUB_AFTER_DENY" <<'PYEOF'
import json, sys
d = json.loads(sys.argv[1])
assert d.get("allow_denied_total", 0) >= 1, d
hosts = d.get("allow_denied_hosts", [])
assert any(
    (isinstance(h, dict) and h.get("host") == "denied.vicanso.dev")
    or (isinstance(h, list) and h and h[0] == "denied.vicanso.dev")
    or (isinstance(h, str) and "denied.vicanso.dev" in h)
    for h in hosts
), f"denied.vicanso.dev missing in allow_denied_hosts: {hosts}"
PYEOF

hub_post '{"op":"reset_allow_denied"}' >/dev/null
HUB_AFTER_RESET="$(curl -sS "$WEB_BASE/api/hub")"
python3 - "$HUB_AFTER_RESET" <<'PYEOF'
import json, sys
d = json.loads(sys.argv[1])
assert d.get("allow_denied_total") == 0, d
assert d.get("allow_denied_hosts") == [], d
PYEOF

# Restore unrestricted allow_hosts for remaining tests.
hub_post '{"op":"set_allow_hosts","text":""}' >/dev/null
pass "allow_hosts 403 incremented allow_denied_total & allow_denied_hosts, reset_allow_denied cleared to 0"

# ==============================================================================
# 9. F2 + F4: Collections export + MCP wait_for_flow, diff_flows, resources, prompts
# ==============================================================================
echo "==> [9] F2 + F4: Collections & MCP tools/resources/prompts"
hub_post "$(python3 -c 'import json,sys; print(json.dumps({"op":"set_rules","text":f"resolve api.vicanso.dev 127.0.0.1:{sys.argv[1]}"}))' "$UPSTREAM_PORT")" >/dev/null

curl -sS -o /dev/null -x "$PROXY_URL" "http://api.vicanso.dev/diff-a"
curl -sS -o /dev/null -x "$PROXY_URL" "http://api.vicanso.dev/diff-b"
curl -sS -o /dev/null -x "$PROXY_URL" "http://api.vicanso.dev/error-500"

# 9a. Collections exports
POSTMAN_OUT="$(curl -sS -w '\n%{http_code}' "$WEB_BASE/api/export.postman")"
BRUNO_OUT="$(curl -sS -w '\n%{http_code}' "$WEB_BASE/api/export.bruno")"
HTTP_OUT="$(curl -sS -w '\n%{http_code}' "$WEB_BASE/api/export.http")"
python3 - "$POSTMAN_OUT" "$BRUNO_OUT" "$HTTP_OUT" <<'PYEOF'
import json, sys

def split_status(raw):
    body, code = raw.rsplit("\n", 1)
    assert code == "200", f"expected HTTP 200, got {code}"
    return body

postman = json.loads(split_status(sys.argv[1]))
assert "info" in postman and "item" in postman and len(postman["item"]) > 0, postman
bruno = split_status(sys.argv[2])
assert "meta {" in bruno and "api.vicanso.dev" in bruno, bruno[:200]
http_txt = split_status(sys.argv[3])
assert "GET http://api.vicanso.dev" in http_txt, http_txt[:200]
PYEOF

# 9b. MCP wait_for_flow (timeout + live arrival)
WAIT_TIMEOUT_RES="$(mcp_tool "wait_for_flow" '{"filter":"path:/never-arrives","timeout_ms":250}')"
python3 - "$WAIT_TIMEOUT_RES" <<'PYEOF'
import json, sys
d = json.loads(json.loads(sys.argv[1])["result"]["content"][0]["text"])
assert d.get("matched") is False, d
PYEOF

LAST_ID="$(curl -sS "$WEB_BASE/api/flows" | python3 -c 'import json,sys; flows=json.load(sys.stdin)["flows"]; print(max(f["id"] for f in flows))')"
mcp_tool "wait_for_flow" "{\"filter\":\"path:/live-wait-hit\",\"timeout_ms\":5000,\"after_id\":$LAST_ID}" >"$TMP_DIR/wait_live.json" &
WAIT_PID=$!
BG_PIDS+=("$WAIT_PID")
sleep 0.3
curl -sS -o /dev/null -x "$PROXY_URL" "http://api.vicanso.dev/live-wait-hit"
wait "$WAIT_PID"
python3 - "$TMP_DIR/wait_live.json" <<'PYEOF'
import json, sys
rpc = json.load(open(sys.argv[1]))
assert not rpc["result"].get("isError"), rpc
d = json.loads(rpc["result"]["content"][0]["text"])
assert d.get("matched") is True and d["flow"]["path"] == "/live-wait-hit", d
PYEOF

# 9c. MCP diff_flows
read -r DIFF_A_ID DIFF_B_ID ERR_500_ID < <(curl -sS "$WEB_BASE/api/flows" | python3 -c '
import json, sys
flows = json.load(sys.stdin)["flows"]
a = [f["id"] for f in flows if f.get("path") == "/diff-a"][-1]
b = [f["id"] for f in flows if f.get("path") == "/diff-b"][-1]
e = [f["id"] for f in flows if f.get("path") == "/error-500"][-1]
print(a, b, e)
')

DIFF_RES="$(mcp_tool "diff_flows" "{\"base_id\":$DIFF_A_ID,\"target_id\":$DIFF_B_ID}")"
python3 - "$DIFF_RES" <<'PYEOF'
import json, sys
rpc = json.loads(sys.argv[1])
assert not rpc["result"].get("isError"), rpc
text = rpc["result"]["content"][0]["text"]
d = json.loads(text)
assert "alpha" in text and "beta" in text, d
PYEOF

# 9d. MCP resources/list & resources/read
RES_LIST="$(mcp_call '{"jsonrpc":"2.0","id":10,"method":"resources/list","params":{}}')"
RES_FLOWS="$(mcp_call '{"jsonrpc":"2.0","id":11,"method":"resources/read","params":{"uri":"zniff://flows/current"}}')"
RES_RULES="$(mcp_call '{"jsonrpc":"2.0","id":12,"method":"resources/read","params":{"uri":"zniff://rules/active"}}')"
python3 - "$RES_LIST" "$RES_FLOWS" "$RES_RULES" <<'PYEOF'
import json, sys
rl = json.loads(sys.argv[1])["result"]["resources"]
uris = {r["uri"] for r in rl}
assert "zniff://flows/current" in uris and "zniff://rules/active" in uris, uris

rf = json.loads(sys.argv[2])["result"]["contents"][0]
assert rf["uri"] == "zniff://flows/current"
rf_data = json.loads(rf["text"])
assert rf_data["session_total"] > 0 and len(rf_data["flows"]) > 0, rf_data

rr = json.loads(sys.argv[3])["result"]["contents"][0]
assert rr["uri"] == "zniff://rules/active"
rr_data = json.loads(rr["text"])
assert "api.vicanso.dev" in rr_data["rules"], rr_data
PYEOF

# 9e. MCP prompts/list & prompts/get
PROMPTS_LIST="$(mcp_call '{"jsonrpc":"2.0","id":20,"method":"prompts/list","params":{}}')"
PROMPT_GET="$(mcp_call "{\"jsonrpc\":\"2.0\",\"id\":21,\"method\":\"prompts/get\",\"params\":{\"name\":\"analyze_failed_flow\",\"arguments\":{\"flow_id\":\"$ERR_500_ID\"}}}")"
python3 - "$PROMPTS_LIST" "$PROMPT_GET" <<'PYEOF'
import json, sys
pl = json.loads(sys.argv[1])["result"]["prompts"]
assert any(p["name"] == "analyze_failed_flow" for p in pl), pl

pg = json.loads(sys.argv[2])["result"]
msgs = pg.get("messages", [])
assert len(msgs) >= 1, pg
combined = " ".join(m.get("content", {}).get("text", "") for m in msgs)
assert "/error-500" in combined and "500" in combined, combined[:400]
PYEOF
pass "F2 + F4: collections exports, wait_for_flow (live+timeout), diff_flows, resources, and prompts verified"

echo ""
echo "======================================================================"
echo "ALL $PASS_COUNT E2E FUNCTIONAL CHECKS PASSED (0 failed)"
echo "======================================================================"
