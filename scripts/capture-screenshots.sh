#!/usr/bin/env bash
# scripts/capture-screenshots.sh
# Automates operational screenshot capture of zniff on macOS at strictly uniform 1920x1080 Full HD (16:9).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DOCS_SCREENSHOT_DIR="$REPO_ROOT/docs/screenshots"
SCREENSHOT_DIR="$REPO_ROOT/screenshots"
TEST_DATA_DIR="/tmp/zniff-screenshot-run"
RAW_DIR="/tmp/zniff-screenshot-raw"
MOCK_SERVER_PORT=19876
PROXY_PORT=18878

mkdir -p "$DOCS_SCREENSHOT_DIR"
mkdir -p "$SCREENSHOT_DIR"
mkdir -p "$RAW_DIR"
rm -rf "$TEST_DATA_DIR"
mkdir -p "$TEST_DATA_DIR"

# Prevent display sleep during automated capture
caffeinate -u -t 120 &
CAFFEINATE_PID=$!

# Query initial macOS dark mode state
ORIGINAL_DARK_MODE=$(osascript -e 'tell application "System Events" to tell appearance preferences to get dark mode' 2>/dev/null || echo "true")
echo "==> Initial system Dark Mode: $ORIGINAL_DARK_MODE"

cleanup() {
    echo "==> Cleaning up background processes..."
    if [ -n "${BREAK_PID1:-}" ] && kill -0 "$BREAK_PID1" 2>/dev/null; then
        kill "$BREAK_PID1" 2>/dev/null || true
    fi
    if [ -n "${BREAK_PID2:-}" ] && kill -0 "$BREAK_PID2" 2>/dev/null; then
        kill "$BREAK_PID2" 2>/dev/null || true
    fi
    if [ -n "${ZNIFF_PID:-}" ] && kill -0 "$ZNIFF_PID" 2>/dev/null; then
        kill "$ZNIFF_PID" 2>/dev/null || true
        wait "$ZNIFF_PID" 2>/dev/null || true
    fi
    if [ -n "${MOCK_PID:-}" ] && kill -0 "$MOCK_PID" 2>/dev/null; then
        kill "$MOCK_PID" 2>/dev/null || true
        wait "$MOCK_PID" 2>/dev/null || true
    fi
    if [ -n "${REC_PID:-}" ] && kill -0 "$REC_PID" 2>/dev/null; then
        kill -INT "$REC_PID" 2>/dev/null || true
        wait "$REC_PID" 2>/dev/null || true
    fi
    if [ -n "${CAFFEINATE_PID:-}" ] && kill -0 "$CAFFEINATE_PID" 2>/dev/null; then
        kill "$CAFFEINATE_PID" 2>/dev/null || true
    fi
    echo "==> Restoring system Dark Mode to: $ORIGINAL_DARK_MODE"
    osascript -e "tell application \"System Events\" to tell appearance preferences to set dark mode to $ORIGINAL_DARK_MODE" 2>/dev/null || true
    rm -rf "$TEST_DATA_DIR" "$RAW_DIR"
    echo "==> Cleanup complete."
}
trap cleanup EXIT INT TERM

set_dark_mode() {
    local enable="$1"
    osascript -e "tell application \"System Events\" to tell appearance preferences to set dark mode to $enable" 2>/dev/null || true
    sleep 0.8
}

get_wid() {
    local pid="$1"
    swift -e 'import CoreGraphics; import Foundation
guard CommandLine.arguments.count > 1, let pid = Int(CommandLine.arguments[1]) else { exit(0) }
let opts = CGWindowListOption(arrayLiteral: .excludeDesktopElements, .optionOnScreenOnly)
if let list = CGWindowListCopyWindowInfo(opts, kCGNullWindowID) as? [[String: Any]] {
    for w in list where (w[kCGWindowOwnerPID as String] as? Int) == pid {
        if let id = w[kCGWindowNumber as String] as? Int { print(id); break }
    }
}' "$pid"
}

get_win_bounds() {
    local pid="$1"
    swift -e 'import CoreGraphics; import Foundation
guard CommandLine.arguments.count > 1, let pid = Int(CommandLine.arguments[1]) else { exit(0) }
let opts = CGWindowListOption(arrayLiteral: .excludeDesktopElements, .optionOnScreenOnly)
if let list = CGWindowListCopyWindowInfo(opts, kCGNullWindowID) as? [[String: Any]] {
    for w in list where (w[kCGWindowOwnerPID as String] as? Int) == pid {
        if let bounds = w[kCGWindowBounds as String] as? [String: Any],
           let x = bounds["X"] as? Double,
           let y = bounds["Y"] as? Double,
           let width = bounds["Width"] as? Double,
           let height = bounds["Height"] as? Double {
            print("\(Int(x)),\(Int(y)),\(Int(width)),\(Int(height))")
            break
        }
    }
}' "$pid"
}

click_win() {
    local pid="$1"
    local rel_x="$2"
    local rel_y="$3"
    osascript -e 'tell application "System Events" to tell process "zniff" to set frontmost to true' 2>/dev/null || true
    sleep 0.1
    swift -e 'import CoreGraphics; import Foundation
guard CommandLine.arguments.count > 3,
      let pid = Int(CommandLine.arguments[1]),
      let relX = Double(CommandLine.arguments[2]),
      let relY = Double(CommandLine.arguments[3]) else { exit(0) }
let opts = CGWindowListOption(arrayLiteral: .excludeDesktopElements, .optionOnScreenOnly)
if let list = CGWindowListCopyWindowInfo(opts, kCGNullWindowID) as? [[String: Any]] {
    for w in list where (w[kCGWindowOwnerPID as String] as? Int) == pid {
        if let bounds = w[kCGWindowBounds as String] as? [String: Any],
           let x = bounds["X"] as? Double,
           let y = bounds["Y"] as? Double {
            let clickPt = CGPoint(x: x + relX, y: y + relY)
            if let down = CGEvent(mouseEventSource: nil, mouseType: .leftMouseDown, mouseCursorPosition: clickPt, mouseButton: .left),
               let up = CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp, mouseCursorPosition: clickPt, mouseButton: .left) {
                down.post(tap: .cghidEventTap)
                usleep(50000)
                up.post(tap: .cghidEventTap)
            }
            break
        }
    }
}' "$pid" "$rel_x" "$rel_y"
    sleep 0.6
}

right_click_win() {
    local pid="$1"
    local rel_x="$2"
    local rel_y="$3"
    osascript -e 'tell application "System Events" to tell process "zniff" to set frontmost to true' 2>/dev/null || true
    sleep 0.1
    swift -e 'import CoreGraphics; import Foundation
guard CommandLine.arguments.count > 3,
      let pid = Int(CommandLine.arguments[1]),
      let relX = Double(CommandLine.arguments[2]),
      let relY = Double(CommandLine.arguments[3]) else { exit(0) }
let opts = CGWindowListOption(arrayLiteral: .excludeDesktopElements, .optionOnScreenOnly)
if let list = CGWindowListCopyWindowInfo(opts, kCGNullWindowID) as? [[String: Any]] {
    for w in list where (w[kCGWindowOwnerPID as String] as? Int) == pid {
        if let bounds = w[kCGWindowBounds as String] as? [String: Any],
           let x = bounds["X"] as? Double,
           let y = bounds["Y"] as? Double {
            let clickPt = CGPoint(x: x + relX, y: y + relY)
            if let down = CGEvent(mouseEventSource: nil, mouseType: .rightMouseDown, mouseCursorPosition: clickPt, mouseButton: .right),
               let up = CGEvent(mouseEventSource: nil, mouseType: .rightMouseUp, mouseCursorPosition: clickPt, mouseButton: .right) {
                down.post(tap: .cghidEventTap)
                usleep(50000)
                up.post(tap: .cghidEventTap)
            }
            break
        }
    }
}' "$pid" "$rel_x" "$rel_y"
    sleep 0.6
}

key_press() {
    local code="$1"
    osascript -e "tell application \"System Events\" to key code $code" 2>/dev/null || true
    sleep 0.4
}

# 1. Start mock backend HTTP server
echo "==> Starting mock backend server on port $MOCK_SERVER_PORT..."
python3 -c "
import http.server, socketserver, json

class MockHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == '/api/v1/users':
            body = json.dumps([
                {'id': 101, 'name': 'Alice Chen', 'email': 'alice@zniff.app', 'role': 'admin'},
                {'id': 102, 'name': 'Bob Smith', 'email': 'bob@zniff.app', 'role': 'engineer'},
                {'id': 103, 'name': 'Carol Wu', 'email': 'carol@zniff.app', 'role': 'designer'}
            ], indent=2).encode('utf-8')
            self.send_response(200)
            self.send_header('Content-Type', 'application/json; charset=utf-8')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif self.path == '/api/v1/metrics':
            self.send_response(304)
            self.send_header('Content-Type', 'text/plain')
            self.send_header('Content-Length', '0')
            self.end_headers()
        elif self.path == '/api/v1/telemetry':
            proto = (
                b'\x08\x96\x01'
                b'\x12\x05zniff'
                b'\x1a\x1btelemetry.v1.ClusterStatus'
                b'\x20\xea\x0f'
            )
            self.send_response(200)
            self.send_header('Content-Type', 'application/x-protobuf')
            self.send_header('Content-Length', str(len(proto)))
            self.end_headers()
            self.wfile.write(proto)
        elif self.path == '/api/v1/billing':
            body = json.dumps({'error': 'InternalServerError', 'message': 'Database connection pool exhausted', 'code': 50012}).encode('utf-8')
            self.send_response(500)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif '/api/v1/breakpoint' in self.path:
            body = json.dumps({'status': 'intercepted', 'path': self.path}).encode('utf-8')
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self):
        length = int(self.headers.get('Content-Length', 0))
        data = self.rfile.read(length) if length > 0 else b''
        if self.path == '/api/graphql':
            body = json.dumps({
                'data': {
                    'user': {
                        'id': 'usr_9482',
                        'name': 'Antigravity Engineer',
                        'email': 'engineer@zniff.app',
                        'role': 'admin',
                        'permissions': ['traffic:capture', 'traffic:replay', 'rules:write'],
                        'profile': {
                            'avatar': 'https://zniff.app/avatars/9482.png',
                            'bio': 'Building high-performance network tools in Rust',
                            'timezone': 'Asia/Shanghai'
                        }
                    }
                }
            }, indent=2).encode('utf-8')
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif self.path == '/api/v1/orders':
            body = json.dumps({'orderId': 'ord_88219', 'status': 'created', 'total': 128.50}).encode('utf-8')
            self.send_response(201)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif '/api/v1/breakpoint' in self.path:
            body = json.dumps({'status': 'intercepted', 'path': self.path}).encode('utf-8')
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_response(200)
            self.end_headers()

    def do_PUT(self):
        body = json.dumps({'id': 101, 'name': 'Alice Chen', 'role': 'superadmin', 'updated': True}).encode('utf-8')
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_DELETE(self):
        self.send_response(204)
        self.end_headers()

    def log_message(self, format, *args):
        pass

socketserver.TCPServer.allow_reuse_address = True
with socketserver.TCPServer(('127.0.0.1', $MOCK_SERVER_PORT), MockHandler) as httpd:
    httpd.serve_forever()
" &
MOCK_PID=$!
sleep 0.5

# 2. Write initial config.dev.toml with capturing=true, 1280x720 window, and breakpoint rule
cat << EOF > "$TEST_DATA_DIR/config.dev.toml"
listen_port = $PROXY_PORT
capturing = true
window_width = 1280
window_height = 720
filter = ""
saved_profiles = []
rules = """
break */api/v1/breakpoint
"""
EOF

# Find zniff binary
TARGET_BIN=""
if [ -x "/Users/tree/cargo-target/debug/zniff" ]; then
    TARGET_BIN="/Users/tree/cargo-target/debug/zniff"
elif [ -x "$REPO_ROOT/target/debug/zniff" ]; then
    TARGET_BIN="$REPO_ROOT/target/debug/zniff"
else
    echo "==> Building zniff binary..."
    cargo build --manifest-path "$REPO_ROOT/app/Cargo.toml"
    TARGET_BIN="$(cargo metadata --manifest-path "$REPO_ROOT/app/Cargo.toml" --format-version 1 | grep -o '"target_directory":"[^"]*"' | cut -d'"' -f4)/debug/zniff"
fi

echo "==> Launching zniff desktop app with binary $TARGET_BIN..."
set_dark_mode true
ZNIFF_DEV=1 "$TARGET_BIN" --data-dir "$TEST_DATA_DIR" &
ZNIFF_PID=$!

echo "==> Waiting for zniff window to appear..."
WID=""
for _ in {1..30}; do
    WID=$(get_wid "$ZNIFF_PID")
    if [ -n "$WID" ]; then
        break
    fi
    sleep 0.5
done

if [ -z "$WID" ]; then
    echo "ERROR: Failed to detect zniff window ID!"
    exit 1
fi
echo "==> Detected zniff window ID: $WID"

# Ensure frontmost and size 1280x720
osascript -e 'tell application "System Events" to tell process "zniff" to set size of window 1 to {1280, 720}' 2>/dev/null || true
osascript -e 'tell application "System Events" to tell process "zniff" to set frontmost to true' 2>/dev/null || true
sleep 1.0

WIN_BOUNDS=$(get_win_bounds "$ZNIFF_PID")
if [ -z "$WIN_BOUNDS" ]; then
    echo "ERROR: Failed to detect zniff window bounds!"
    exit 1
fi
RAW_VIDEO="$RAW_DIR/zniff_raw_recording.mov"
echo "==> Starting real-time window recording ($WIN_BOUNDS)..."
/usr/sbin/screencapture -x -v -C -k -R "$WIN_BOUNDS" "$RAW_VIDEO" &
REC_PID=$!
sleep 0.8

capture_uniform_screenshot() {
    local scene_name="$1"
    local raw_path="$RAW_DIR/raw_${scene_name}"
    local docs_path="$DOCS_SCREENSHOT_DIR/${scene_name}"
    local mirror_path="$SCREENSHOT_DIR/${scene_name}"

    echo "==> Capturing $scene_name (shadowless 1280x720 canvas)..."
    /usr/sbin/screencapture -x -l "$WID" -o "$raw_path"
    sips -z 1080 1920 "$raw_path" --out "$docs_path" >/dev/null
    cp -f "$docs_path" "$mirror_path"
    echo "    Saved: $docs_path (1920x1080)"
}

# -------------------------------------------------------------
# Inject rich mock traffic flows
# -------------------------------------------------------------
echo "==> Injecting rich mock traffic flows..."
PROXY="http://127.0.0.1:$PROXY_PORT"
MOCK="http://127.0.0.1:$MOCK_SERVER_PORT"

# Flow 1: Users JSON (GET 200)
curl -s -x "$PROXY" "$MOCK/api/v1/users" > /dev/null
# Flow 2: Orders JSON (POST 201)
curl -s -x "$PROXY" -X POST "$MOCK/api/v1/orders" -H "Content-Type: application/json" -d '{"items":["zniff-pro"],"total":128.50}' > /dev/null
# Flow 3: Update User (PUT 200)
curl -s -x "$PROXY" -X PUT "$MOCK/api/v1/users/101" -H "Content-Type: application/json" -d '{"role":"superadmin"}' > /dev/null
# Flow 4: Session Delete (DELETE 204)
curl -s -x "$PROXY" -X DELETE "$MOCK/api/v1/sessions/current" > /dev/null
# Flow 5: GraphQL Query (POST 200)
GQL_PAYLOAD='{"operationName":"GetUserProfile","query":"query GetUserProfile($id: ID!) {\n  user(id: $id) {\n    id\n    name\n    email\n    role\n    permissions\n    profile {\n      avatar\n      bio\n      timezone\n    }\n  }\n}","variables":{"id":"usr_9482"}}'
curl -s -x "$PROXY" -X POST "$MOCK/api/graphql" -H "Content-Type: application/json" -d "$GQL_PAYLOAD" > /dev/null
# Flow 6: Plaintext Metrics (GET 304)
curl -s -x "$PROXY" "$MOCK/api/v1/metrics" > /dev/null
# Flow 7: Protobuf Telemetry (GET 200)
curl -s -x "$PROXY" "$MOCK/api/v1/telemetry" > /dev/null
# Flow 8: Server Billing Error (GET 500)
curl -s -x "$PROXY" "$MOCK/api/v1/billing" > /dev/null || true
# Flow 9: Tunnel Failure (ERR)
curl -s -x "$PROXY" "http://127.0.0.1:19999/unreachable/service" > /dev/null || true
# Flow 10: AI-Native MCP tools list (POST /mcp 200)
MCP_REQ='{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
curl -s -x "$PROXY" -X POST "http://127.0.0.1:$PROXY_PORT/mcp" -H "Content-Type: application/json" -d "$MCP_REQ" > /dev/null

sleep 1.5

# -------------------------------------------------------------
# Scene 01: 01_traffic_table_dual_pane.png
# -------------------------------------------------------------
echo "==> Preparing Scene 01: Main Dual-Pane Traffic Table..."
key_press 53 # Escape ensures no row selected and no popover open
sleep 0.6
capture_uniform_screenshot "01_traffic_table_dual_pane.png"

# -------------------------------------------------------------
# Scene 02: 02_detail_json_graphql.png
# -------------------------------------------------------------
echo "==> Preparing Scene 02: Deep Inspection JSON & GraphQL..."
# Row 5 (GraphQL at y=261)
click_win "$ZNIFF_PID" 200 261
sleep 0.8
capture_uniform_screenshot "02_detail_json_graphql.png"

# -------------------------------------------------------------
# Scene 03: 03_detail_protobuf_hex.png
# -------------------------------------------------------------
echo "==> Preparing Scene 03: Protobuf / gRPC & Hex View..."
# Row 7 (Telemetry at y=329)
click_win "$ZNIFF_PID" 200 329
sleep 0.8
capture_uniform_screenshot "03_detail_protobuf_hex.png"

# -------------------------------------------------------------
# Scene 04: 04_baseline_diff_view.png
# -------------------------------------------------------------
echo "==> Preparing Scene 04: Line-by-Line Baseline Diff View..."
# Right click Row 1 to open context menu
right_click_win "$ZNIFF_PID" 200 125
sleep 0.5
# Click '基准' (Baseline) at relative y=300
click_win "$ZNIFF_PID" 260 300
sleep 0.6
# Left click Row 3 to trigger diff against Row 1
click_win "$ZNIFF_PID" 200 193
sleep 0.8
capture_uniform_screenshot "04_baseline_diff_view.png"

# Clear baseline
right_click_win "$ZNIFF_PID" 200 125
sleep 0.5
click_win "$ZNIFF_PID" 260 300
sleep 0.5

# -------------------------------------------------------------
# Scene 05: 05_filter_rules_popup.png
# -------------------------------------------------------------
echo "==> Preparing Scene 05: Filter & Rules Management Popover..."
# Click '过滤' in toolbar (x=985, y=21)
click_win "$ZNIFF_PID" 985 21
sleep 0.8
capture_uniform_screenshot "05_filter_rules_popup.png"
# Close popover with Escape
key_press 53
sleep 0.5

# -------------------------------------------------------------
# Scene 06: 06_breakpoint_intercept_queue.png
# -------------------------------------------------------------
echo "==> Preparing Scene 06: Breakpoint Interception & Concurrency Queue..."
# Send two background requests that will pause on breakpoint
curl -s -x "$PROXY" "$MOCK/api/v1/breakpoint/user-auth" >/dev/null &
BREAK_PID1=$!
sleep 0.3
curl -s -x "$PROXY" "$MOCK/api/v1/breakpoint/payment-checkout" >/dev/null &
BREAK_PID2=$!
sleep 1.2
capture_uniform_screenshot "06_breakpoint_intercept_queue.png"

# Resume/drop breakpoints by continuing or killing
kill "$BREAK_PID1" "$BREAK_PID2" 2>/dev/null || true
key_press 53
sleep 0.5

# -------------------------------------------------------------
# Scene 07: 07_sequence_diagram_waterfall.png
# -------------------------------------------------------------
echo "==> Preparing Scene 07: Multi-View Sequence & Waterfall Preview..."
# Click More menu in toolbar (x=1150, y=21)
click_win "$ZNIFF_PID" 1150 21
sleep 0.5
# Click '生成时序图' (x=1150, y=164)
click_win "$ZNIFF_PID" 1150 164
sleep 1.0
capture_uniform_screenshot "07_sequence_diagram_waterfall.png"
# Close dialog with Escape
key_press 53
sleep 0.5

# -------------------------------------------------------------
# Scene 08: 08_mobile_qr_cert_console.png
# -------------------------------------------------------------
echo "==> Preparing Scene 08: Mobile QR Code & CA Trust Management..."
# Click More menu in toolbar (x=1150, y=21)
click_win "$ZNIFF_PID" 1150 21
sleep 0.8
capture_uniform_screenshot "08_mobile_qr_cert_console.png"
# Close menu with Escape
key_press 53
sleep 0.5

# -------------------------------------------------------------
# Scene 09: 09_mcp_ai_integration.png
# -------------------------------------------------------------
echo "==> Preparing Scene 09: AI-Native MCP Server Integration..."
# Select MCP flow row (latest row at y=431)
click_win "$ZNIFF_PID" 200 431
sleep 0.8
# Copy MCP config from More menu to display status notification
click_win "$ZNIFF_PID" 1150 21 # More menu
sleep 0.5
click_win "$ZNIFF_PID" 1230 325 # MCP pill in Copy Config
sleep 0.6
capture_uniform_screenshot "09_mcp_ai_integration.png"

# -------------------------------------------------------------
# Scene 10: 10_theme_light_contrast.png
# -------------------------------------------------------------
echo "==> Preparing Scene 10: Adaptive Light Mode & High-Contrast Layout..."
set_dark_mode false
sleep 1.0
key_press 53 # Escape to summary view
sleep 0.5
capture_uniform_screenshot "10_theme_light_contrast.png"
set_dark_mode true
sleep 0.5

# Stop real-time recording cleanly
if [ -n "${REC_PID:-}" ] && kill -0 "$REC_PID" 2>/dev/null; then
    echo "==> Stopping real-time window recording..."
    kill -INT "$REC_PID" 2>/dev/null || true
    wait "$REC_PID" 2>/dev/null || true
    unset REC_PID
fi

# -------------------------------------------------------------
# Clean up old outdated non-uniform screenshots in docs/screenshots
# -------------------------------------------------------------
echo "==> Pruning any unused files in $DOCS_SCREENSHOT_DIR..."
for f in "$DOCS_SCREENSHOT_DIR"/*.png; do
    fname="$(basename "$f")"
    case "$fname" in
        02_detail_json_graphql.png|\
        05_filter_rules_popup.png|\
        07_sequence_diagram_waterfall.png|\
        10_theme_light_contrast.png)
            # Retain screenshots referenced by README
            ;;
        *)
            rm -f "$f"
            ;;
    esac
done

echo "==> Pruning any legacy non-uniform files in $SCREENSHOT_DIR..."
for f in "$SCREENSHOT_DIR"/*.png; do
    fname="$(basename "$f")"
    case "$fname" in
        01_traffic_table_dual_pane.png|\
        02_detail_json_graphql.png|\
        03_detail_protobuf_hex.png|\
        04_baseline_diff_view.png|\
        05_filter_rules_popup.png|\
        06_breakpoint_intercept_queue.png|\
        07_sequence_diagram_waterfall.png|\
        08_mobile_qr_cert_console.png|\
        09_mcp_ai_integration.png|\
        10_theme_light_contrast.png)
            # Retain current 10 scenes
            ;;
        *)
            rm -f "$f"
            ;;
    esac
done

# -------------------------------------------------------------
# Verify Requirement R2: Strict Uniform Resolution (1920x1080)
# -------------------------------------------------------------
echo "==> Verifying Requirement R2: Dimensions across all screenshots..."
for dir in "$DOCS_SCREENSHOT_DIR" "$SCREENSHOT_DIR"; do
    echo "Checking directory: $dir"
    for img in "$dir"/*.png; do
        w=$(sips -g pixelWidth "$img" | awk '/pixelWidth/ {print $2}')
        h=$(sips -g pixelHeight "$img" | awk '/pixelHeight/ {print $2}')
        if [ "$w" != "1920" ] || [ "$h" != "1080" ]; then
            echo "ERROR: Image $img has dimension ${w}x${h}, expected 1920x1080!"
            exit 1
        fi
        echo "  [PASS] $(basename "$img"): ${w}x${h}"
    done
done

# -------------------------------------------------------------
# Generate 1920x1080 bilingual intro card and transcode video
# -------------------------------------------------------------
INTRO_PNG="$RAW_DIR/zniff_intro.png"
echo "==> Generating 1920x1080 bilingual intro card..."
swift - "$REPO_ROOT/icons/zniff.icns" "$INTRO_PNG" <<'SWIFT'
import AppKit
import Foundation

let iconPath = CommandLine.arguments[1]
let outPath = CommandLine.arguments[2]
let W = 1920
let H = 1080

guard let rep = NSBitmapImageRep(
    bitmapDataPlanes: nil,
    pixelsWide: W,
    pixelsHigh: H,
    bitsPerSample: 8,
    samplesPerPixel: 4,
    hasAlpha: true,
    isPlanar: false,
    colorSpaceName: .deviceRGB,
    bytesPerRow: 0,
    bitsPerPixel: 0
) else { exit(1) }

NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)

let rect = NSRect(x: 0, y: 0, width: W, height: H)
let bgGrad = NSGradient(
    starting: NSColor(red: 0.043, green: 0.067, blue: 0.118, alpha: 1.0),
    ending: NSColor(red: 0.067, green: 0.106, blue: 0.180, alpha: 1.0)
)
bgGrad?.draw(in: rect, angle: 90)

// Subtle radial glow behind icon/title
if let glow = NSGradient(
    colorsAndLocations:
        (NSColor(red: 0.231, green: 0.510, blue: 0.965, alpha: 0.14), 0.0),
        (NSColor(red: 0.231, green: 0.510, blue: 0.965, alpha: 0.0), 1.0)
) {
    glow.draw(in: NSRect(x: W / 2 - 480, y: H / 2 - 220, width: 960, height: 760), relativeCenterPosition: .zero)
}

func drawCentered(_ text: String, y: CGFloat, font: NSFont, color: NSColor) {
    let style = NSMutableParagraphStyle()
    style.alignment = .center
    let attrs: [NSAttributedString.Key: Any] = [
        .font: font,
        .foregroundColor: color,
        .paragraphStyle: style,
    ]
    let str = NSAttributedString(string: text, attributes: attrs)
    let size = str.size()
    str.draw(in: NSRect(x: 0, y: y, width: CGFloat(W), height: size.height + 8))
}

// App Icon
if let icon = NSImage(contentsOfFile: iconPath) {
    let iconSize: CGFloat = 148
    icon.draw(in: NSRect(x: (CGFloat(W) - iconSize) / 2, y: 690, width: iconSize, height: iconSize))
}

// Title & bilingual taglines (AppKit y=0 is bottom)
drawCentered(
    "zniff",
    y: 575,
    font: NSFont.systemFont(ofSize: 84, weight: .bold),
    color: NSColor(red: 0.973, green: 0.980, blue: 0.988, alpha: 1.0)
)
drawCentered(
    "Single-process HTTP(S) debugging proxy in pure Rust",
    y: 505,
    font: NSFont.systemFont(ofSize: 30, weight: .medium),
    color: NSColor(red: 0.800, green: 0.847, blue: 0.910, alpha: 1.0)
)
drawCentered(
    "基于纯 Rust 的单进程 HTTP(S) 抓包调试代理",
    y: 452,
    font: NSFont.systemFont(ofSize: 27, weight: .regular),
    color: NSColor(red: 0.580, green: 0.639, blue: 0.722, alpha: 1.0)
)

// Feature pills row
let pills = [
    "Native GUI · 原生界面",
    "HTTP/2 & WebSocket",
    "14 Rules · 14 项改写规则",
    "Built-in MCP · AI 原生 MCP",
]
let pillFont = NSFont.systemFont(ofSize: 20, weight: .medium)
let pillAttrs: [NSAttributedString.Key: Any] = [
    .font: pillFont,
    .foregroundColor: NSColor(red: 0.886, green: 0.918, blue: 0.961, alpha: 1.0),
]
let padX: CGFloat = 24
let pillH: CGFloat = 46
let gap: CGFloat = 18
let widths = pills.map { NSAttributedString(string: $0, attributes: pillAttrs).size().width + padX * 2 }
let totalW = widths.reduce(0, +) + gap * CGFloat(pills.count - 1)
var curX = (CGFloat(W) - totalW) / 2
let pillY: CGFloat = 338

for (i, label) in pills.enumerated() {
    let pw = widths[i]
    let pRect = NSRect(x: curX, y: pillY, width: pw, height: pillH)
    let path = NSBezierPath(roundedRect: pRect, xRadius: 23, yRadius: 23)
    NSColor(red: 0.094, green: 0.137, blue: 0.216, alpha: 0.95).setFill()
    path.fill()
    NSColor(red: 0.180, green: 0.255, blue: 0.376, alpha: 1.0).setStroke()
    path.lineWidth = 1.5
    path.stroke()

    let str = NSAttributedString(string: label, attributes: pillAttrs)
    let sz = str.size()
    str.draw(at: NSPoint(x: curX + (pw - sz.width) / 2, y: pillY + (pillH - sz.height) / 2))
    curX += pw + gap
}

drawCentered(
    "Live Capture & Inspection Demo  ·  实时抓包与调试演示",
    y: 130,
    font: NSFont.systemFont(ofSize: 20, weight: .regular),
    color: NSColor(red: 0.392, green: 0.455, blue: 0.545, alpha: 1.0)
)

NSGraphicsContext.restoreGraphicsState()
if let png = rep.representation(using: .png, properties: [:]) {
    try png.write(to: URL(fileURLWithPath: outPath))
}
SWIFT

echo "==> Combining intro card + real-time recording via ffmpeg..."
DEMO_VIDEO="$SCREENSHOT_DIR/zniff_demo.mp4"
TEST_VIDEO="/tmp/zniff_showcase_preview.mp4"
rm -f "$DEMO_VIDEO" "$TEST_VIDEO"
ffmpeg -y \
    -loop 1 -t 2.4 -framerate 30 -i "$INTRO_PNG" \
    -i "$RAW_VIDEO" \
    -filter_complex "[0:v]scale=1920:1080,setsar=1,format=yuv420p[intro];[1:v]scale=1920:1080:flags=lanczos,fps=30,setsar=1,format=yuv420p[main];[intro][main]xfade=transition=fade:duration=0.4:offset=2.0,format=yuv420p[v]" \
    -map "[v]" -c:v libx264 -preset medium -crf 22 -movflags +faststart \
    "$DEMO_VIDEO"
cp -f "$DEMO_VIDEO" "$TEST_VIDEO"
echo "==> Demo video generated at $DEMO_VIDEO ($(ls -lh "$DEMO_VIDEO" | awk '{print $5}'))"

echo "==> Capture & recording pipeline completed successfully!"
