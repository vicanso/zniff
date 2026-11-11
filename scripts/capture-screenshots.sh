#!/usr/bin/env bash
# scripts/capture-screenshots.sh
# Automates operational screenshot capture of zniff on macOS.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SCREENSHOT_DIR="$REPO_ROOT/screenshots"
TEST_DATA_DIR="/tmp/zniff-screenshot-run"
MOCK_SERVER_PORT=19876
PROXY_PORT=8878

mkdir -p "$SCREENSHOT_DIR"
rm -rf "$TEST_DATA_DIR"
mkdir -p "$TEST_DATA_DIR"

# Query initial macOS dark mode state
ORIGINAL_DARK_MODE=$(osascript -e 'tell application "System Events" to tell appearance preferences to get dark mode' 2>/dev/null || echo "true")
echo "==> Initial system Dark Mode: $ORIGINAL_DARK_MODE"

cleanup() {
    echo "==> Cleaning up background processes..."
    if [ -n "${ZNIFF_PID:-}" ] && kill -0 "$ZNIFF_PID" 2>/dev/null; then
        kill "$ZNIFF_PID" 2>/dev/null || true
        wait "$ZNIFF_PID" 2>/dev/null || true
    fi
    if [ -n "${MOCK_PID:-}" ] && kill -0 "$MOCK_PID" 2>/dev/null; then
        kill "$MOCK_PID" 2>/dev/null || true
        wait "$MOCK_PID" 2>/dev/null || true
    fi
    echo "==> Restoring system Dark Mode to: $ORIGINAL_DARK_MODE"
    osascript -e "tell application \"System Events\" to tell appearance preferences to set dark mode to $ORIGINAL_DARK_MODE" 2>/dev/null || true
    rm -rf "$TEST_DATA_DIR"
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
    swift -e "import CoreGraphics; import Foundation
let opts = CGWindowListOption(arrayLiteral: .excludeDesktopElements, .optionOnScreenOnly)
if let list = CGWindowListCopyWindowInfo(opts, kCGNullWindowID) as? [[String: Any]] {
    for w in list where (w[kCGWindowOwnerPID as String] as? Int) == $pid {
        if let id = w[kCGWindowNumber as String] as? Int { print(id); break }
    }
}"
}

click_win() {
    local pid="$1"
    local rel_x="$2"
    local rel_y="$3"
    osascript -e 'tell application "System Events" to tell process "zniff" to set frontmost to true' 2>/dev/null || true
    sleep 0.2
    swift -e "import CoreGraphics; import Foundation
let opts = CGWindowListOption(arrayLiteral: .excludeDesktopElements, .optionOnScreenOnly)
if let list = CGWindowListCopyWindowInfo(opts, kCGNullWindowID) as? [[String: Any]] {
    for w in list where (w[kCGWindowOwnerPID as String] as? Int) == $pid {
        if let bounds = w[kCGWindowBounds as String] as? [String: Any],
           let x = bounds[\"X\"] as? Double,
           let y = bounds[\"Y\"] as? Double {
            let clickPt = CGPoint(x: x + Double($rel_x), y: y + Double($rel_y))
            if let down = CGEvent(mouseEventSource: nil, mouseType: .leftMouseDown, mouseCursorPosition: clickPt, mouseButton: .left),
               let up = CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp, mouseCursorPosition: clickPt, mouseButton: .left) {
                down.post(tap: .cghidEventTap)
                usleep(50000)
                up.post(tap: .cghidEventTap)
            }
            break
        }
    }
}"
    sleep 0.6
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
            body = b'# HELP http_requests_total Total requests\nhttp_requests_total{status=\"200\"} 1420\n'
            self.send_response(200)
            self.send_header('Content-Type', 'text/plain; charset=utf-8')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif self.path == '/api/v1/error':
            body = json.dumps({'error': 'InternalServerError', 'message': 'Database connection pool exhausted'}).encode('utf-8')
            self.send_response(500)
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
                    'getUser': {
                        'id': 'usr_9482',
                        'name': 'Antigravity Lead',
                        'permissions': ['capture', 'decrypt', 'replay'],
                        'status': 'ACTIVE'
                    }
                }
            }, indent=2).encode('utf-8')
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif self.path == '/api/v1/orders':
            body = json.dumps({'orderId': 'ord_88219', 'status': 'created'}).encode('utf-8')
            self.send_response(201)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_response(200)
            self.end_headers()

    def log_message(self, format, *args):
        pass

socketserver.TCPServer.allow_reuse_address = True
with socketserver.TCPServer(('127.0.0.1', $MOCK_SERVER_PORT), MockHandler) as httpd:
    httpd.serve_forever()
" &
MOCK_PID=$!
sleep 0.5

# 2. Write initial config.dev.toml with capturing=true
cat << EOF > "$TEST_DATA_DIR/config.dev.toml"
listen_port = $PROXY_PORT
capturing = true
window_width = 1200
window_height = 760
filter = ""
saved_profiles = []
EOF

# Ensure binary is built
echo "==> Building zniff binary..."
cargo build --manifest-path "$REPO_ROOT/app/Cargo.toml"

echo "==> Launching zniff desktop app..."
ZNIFF_DEV=1 /Users/tree/cargo-target/debug/zniff --data-dir "$TEST_DATA_DIR" &
ZNIFF_PID=$!

echo "==> Waiting for zniff window to appear..."
WID=""
for _ in {1..20}; do
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

osascript -e 'tell application "System Events" to tell process "zniff" to set frontmost to true' 2>/dev/null || true
sleep 1.0

# -------------------------------------------------------------
# 01_initial_launch_dark.png
# -------------------------------------------------------------
echo "==> Capturing 01_initial_launch_dark.png..."
set_dark_mode true
/usr/sbin/screencapture -x -l "$WID" "$SCREENSHOT_DIR/01_initial_launch_dark.png"

# -------------------------------------------------------------
# 02_initial_launch_light.png
# -------------------------------------------------------------
echo "==> Capturing 02_initial_launch_light.png..."
set_dark_mode false
/usr/sbin/screencapture -x -l "$WID" "$SCREENSHOT_DIR/02_initial_launch_light.png"
set_dark_mode true

# -------------------------------------------------------------
# Inject mock traffic
# -------------------------------------------------------------
echo "==> Injecting rich mock traffic flows..."
# Flow 1: Users JSON
curl -s -x http://127.0.0.1:$PROXY_PORT http://127.0.0.1:$MOCK_SERVER_PORT/api/v1/users > /dev/null
# Flow 2: GraphQL query
curl -s -x http://127.0.0.1:$PROXY_PORT -X POST http://127.0.0.1:$MOCK_SERVER_PORT/api/graphql \
  -H "Content-Type: application/json" \
  -d '{"operationName":"GetUser","query":"query GetUser { user { id name email } }"}' > /dev/null
# Flow 3: CA install page (HTML)
curl -s -x http://127.0.0.1:$PROXY_PORT http://127.0.0.1:$PROXY_PORT/zniff > /dev/null
# Flow 4: Metrics (Plaintext)
curl -s -x http://127.0.0.1:$PROXY_PORT http://127.0.0.1:$MOCK_SERVER_PORT/api/v1/metrics > /dev/null
# Flow 5: Order creation (201 Created)
curl -s -x http://127.0.0.1:$PROXY_PORT -X POST http://127.0.0.1:$MOCK_SERVER_PORT/api/v1/orders \
  -H "Content-Type: application/json" \
  -d '{"items":["item_a","item_b"],"total":88.5}' > /dev/null
# Flow 6: Server error (500)
curl -s -x http://127.0.0.1:$PROXY_PORT http://127.0.0.1:$MOCK_SERVER_PORT/api/v1/error > /dev/null || true
# Flow 7: Unreachable origin (ERR tunnel failure)
curl -s -x http://127.0.0.1:$PROXY_PORT http://127.0.0.1:19999/unreachable > /dev/null || true

sleep 1.2

# -------------------------------------------------------------
# 03_active_traffic_table.png
# -------------------------------------------------------------
echo "==> Capturing 03_active_traffic_table.png..."
/usr/sbin/screencapture -x -l "$WID" "$SCREENSHOT_DIR/03_active_traffic_table.png"

# -------------------------------------------------------------
# 04_search_filtering_and_facets.png
# -------------------------------------------------------------
echo "==> Capturing 04_search_filtering_and_facets.png..."
# Open facets drawer (Icon 1 at x=602, y=62)
click_win "$ZNIFF_PID" 602 62
sleep 0.5
/usr/sbin/screencapture -x -l "$WID" "$SCREENSHOT_DIR/04_search_filtering_and_facets.png"
# Close facets drawer
click_win "$ZNIFF_PID" 602 62
sleep 0.4

# -------------------------------------------------------------
# 05_detail_inspector_overview.png
# -------------------------------------------------------------
echo "==> Capturing 05_detail_inspector_overview.png..."
# Select row 2 (GraphQL flow at y=164)
click_win "$ZNIFF_PID" 200 164
sleep 0.5
/usr/sbin/screencapture -x -l "$WID" "$SCREENSHOT_DIR/05_detail_inspector_overview.png"

# -------------------------------------------------------------
# 06_detail_body_and_headers.png
# -------------------------------------------------------------
echo "==> Capturing 06_detail_body_and_headers.png..."
# Select row 1 (Users JSON flow at y=130)
click_win "$ZNIFF_PID" 200 130
sleep 0.5
/usr/sbin/screencapture -x -l "$WID" "$SCREENSHOT_DIR/06_detail_body_and_headers.png"

# -------------------------------------------------------------
# 07_rule_management_popup.png
# -------------------------------------------------------------
echo "==> Capturing 07_rule_management_popup.png..."
# Click Filters button in toolbar (x=985, y=21)
click_win "$ZNIFF_PID" 985 21
sleep 0.8
/usr/sbin/screencapture -x -l "$WID" "$SCREENSHOT_DIR/07_rule_management_popup.png"
# Close popup with Escape
osascript -e 'tell application "System Events" to key code 53' 2>/dev/null || true
sleep 0.5

# -------------------------------------------------------------
# 08_sequence_diagram_preview.png
# -------------------------------------------------------------
echo "==> Capturing 08_sequence_diagram_preview.png..."
# Deselect row to return to Summary view: click "Summary" button at (x=840, y=21)
click_win "$ZNIFF_PID" 840 21
sleep 0.5
# Click "Sequence diagram" button in summary card (x=1110, y=74)
click_win "$ZNIFF_PID" 1110 74
sleep 0.8
/usr/sbin/screencapture -x -l "$WID" "$SCREENSHOT_DIR/08_sequence_diagram_preview.png"
# Close modal with Escape
osascript -e 'tell application "System Events" to key code 53' 2>/dev/null || true
sleep 0.5

# -------------------------------------------------------------
# 09_window_resizing_compact.png
# -------------------------------------------------------------
echo "==> Capturing 09_window_resizing_compact.png..."
osascript -e 'tell application "System Events" to tell process "zniff" to set size of window 1 to {800, 600}' 2>/dev/null || true
sleep 0.8
/usr/sbin/screencapture -x -l "$WID" "$SCREENSHOT_DIR/09_window_resizing_compact.png"

# -------------------------------------------------------------
# 10_window_resizing_wide.png
# -------------------------------------------------------------
echo "==> Capturing 10_window_resizing_wide.png..."
osascript -e 'tell application "System Events" to tell process "zniff" to set size of window 1 to {1440, 800}' 2>/dev/null || true
sleep 0.8
/usr/sbin/screencapture -x -l "$WID" "$SCREENSHOT_DIR/10_window_resizing_wide.png"

echo "==> All 10 screenshots captured successfully in $SCREENSHOT_DIR:"
ls -lh "$SCREENSHOT_DIR"/*.png
