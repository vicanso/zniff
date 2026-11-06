#!/bin/sh
# Send sample HTTP/HTTPS traffic through a running zniff proxy to exercise UI
# cards, badges, annotations, image preview, hex reflow, decompression, SSE,
# slow-request warnings, and TLS error handling.
#
# Usage:
#   scripts/test-traffic.sh                # run all scenarios via 127.0.0.1:8877
#   scripts/test-traffic.sh --dev          # run all scenarios via 127.0.0.1:8878 (dev config)
#   scripts/test-traffic.sh --proxy HOST:PORT
#   scripts/test-traffic.sh <group>        # json | methods | form | cookie | image | hex | encoding | slow | status | sse | tls | local
set -eu

PROXY="${ZNIFF_PROXY:-http://127.0.0.1:8877}"
GROUP="all"

while [ $# -gt 0 ]; do
  case "$1" in
    --dev)
      PROXY="http://127.0.0.1:8878"
      shift
      ;;
    --proxy|-x)
      PROXY="$2"
      shift 2
      ;;
    -h|--help)
      sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      GROUP="$1"
      shift
      ;;
  esac
done

case "$(uname -s)" in
  Darwin) DEFAULT_CA="$HOME/Library/Application Support/zniff/ca.crt" ;;
  *)      DEFAULT_CA="${XDG_DATA_HOME:-$HOME/.local/share}/zniff/ca.crt" ;;
esac

CA_CERT="${ZNIFF_CA_CERT:-$DEFAULT_CA}"
if [ -f "$CA_CERT" ]; then
  TLS_ARGS="--cacert"
else
  TLS_ARGS="-k"
  CA_CERT=""
fi

run_curl() {
  label=$1
  shift
  printf '==> %s\n' "$label"
  if [ -n "$CA_CERT" ]; then
    curl -sS -o /dev/null -w "    status=%{http_code} time=%{time_total}s size=%{size_download}B\n" \
      -x "$PROXY" "$TLS_ARGS" "$CA_CERT" --max-time 15 "$@" || true
  else
    curl -sS -o /dev/null -w "    status=%{http_code} time=%{time_total}s size=%{size_download}B\n" \
      -x "$PROXY" -k --max-time 15 "$@" || true
  fi
}

# Sample JWT: header {"alg":"HS256","typ":"JWT"}, payload {"sub":"10086","name":"zniff-user","iat":1730905200,"exp":1893456000}
SAMPLE_JWT="eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMDA4NiIsIm5hbWUiOiJ6bmlmZi11c2VyIiwiaWF0IjoxNzMwOTA1MjAwLCJleHAiOjE4OTM0NTYwMDB9.c2lnbmF0dXJl"
# Sample printable Base64 ("zniff-debug-payload-2026")
SAMPLE_B64="em5pZmYtZGVidWctcGF5bG9hZC0yMDI2"

# 1. Local zniff install page & root certificate
scenario_local() {
  run_curl "[local] Mobile install page (/zniff)" \
    "http://127.0.0.1:${PROXY##*:}/zniff" -H "Accept-Language: zh-CN,zh;q=0.9"
  run_curl "[local] Root CA PEM download (/zniff-ca.crt)" \
    "http://127.0.0.1:${PROXY##*:}/zniff-ca.crt"
}

# 2. Query parameters + JWT / Unix timestamp / Base64 annotations + JSON pretty view
scenario_json() {
  run_curl "[json] GET with Query params, JWT, timestamp, and Base64 annotations" \
    "https://httpbin.org/get?env=staging&ts=1730905200&token=${SAMPLE_B64}&long_path_segment=alpha-bravo-charlie-delta-echo-foxtrot" \
    -H "Authorization: Bearer ${SAMPLE_JWT}" \
    -H "X-Request-Timestamp: 1730905200000" \
    -H "X-Encoded-Meta: ${SAMPLE_B64}"
}

# 3. RESTful methods (GET / POST / PUT / PATCH / DELETE) & TypeScript interface generation
scenario_methods() {
  run_curl "[methods] GET JSON list (jsonplaceholder)" \
    "https://jsonplaceholder.typicode.com/posts?_limit=3"
  run_curl "[methods] POST JSON body" \
    -X POST "https://jsonplaceholder.typicode.com/posts" \
    -H "Content-Type: application/json" \
    -d '{"title":"zniff capture test","body":"testing POST badge and JSON body","userId":1}'
  run_curl "[methods] PUT JSON body" \
    -X PUT "https://jsonplaceholder.typicode.com/posts/1" \
    -H "Content-Type: application/json" \
    -d '{"id":1,"title":"zniff updated title","body":"testing PUT badge and diff baseline","userId":1}'
  run_curl "[methods] PATCH JSON body" \
    -X PATCH "https://jsonplaceholder.typicode.com/posts/1" \
    -H "Content-Type: application/json" \
    -d '{"title":"zniff patched title"}'
  run_curl "[methods] DELETE resource" \
    -X DELETE "https://jsonplaceholder.typicode.com/posts/1"
}

# 4. URL-encoded form body (key-value table + inline JWT/timestamp/Base64 annotations)
scenario_form() {
  run_curl "[form] POST application/x-www-form-urlencoded" \
    -X POST "https://postman-echo.com/post?region=ap-east-1" \
    -H "Content-Type: application/x-www-form-urlencoded" \
    -d "client=zniff+desktop&issued_at=1730905200&payload=${SAMPLE_B64}&jwt=${SAMPLE_JWT}"
}

# 5. Cookie / Set-Cookie splitting & header sorting
scenario_cookie() {
  run_curl "[cookie] Cookie & Set-Cookie header splitting" \
    "https://httpbin.org/response-headers?Set-Cookie=sid%3D${SAMPLE_B64}%3B%20Path%3D%2F%3B%20HttpOnly%3B%20Secure" \
    -H "Cookie: theme=dark; lang=zh-CN; auth_jwt=${SAMPLE_JWT}; login_at=1730905200"
}

# 6. Image responses (PNG / JPEG / WebP preview scaling + Raw 16/32-byte Hex toggle)
scenario_image() {
  run_curl "[image] PNG image preview & Raw Hex toggle" \
    "https://httpbin.org/image/png"
  run_curl "[image] JPEG image preview" \
    "https://httpbin.org/image/jpeg"
  run_curl "[image] WebP image preview" \
    "https://httpbin.org/image/webp"
}

# 7. Binary Hex body (4-digit row numbers, 16/32-byte responsive width)
scenario_hex() {
  run_curl "[hex] Binary 128-byte response body" \
    "https://httpbin.org/bytes/128"
}

# 8. Content-Encoding decompression (gzip / brotli / deflate)
scenario_encoding() {
  run_curl "[encoding] Gzip compressed JSON (httpbin.org/gzip)" \
    -H "Accept-Encoding: gzip" "https://httpbin.org/gzip"
  run_curl "[encoding] Gzip compressed JSON (postman-echo.com/gzip)" \
    -H "Accept-Encoding: gzip" "https://postman-echo.com/gzip"
  run_curl "[encoding] Brotli compressed JSON (httpbin.org/brotli)" \
    -H "Accept-Encoding: br" "https://httpbin.org/brotli"
  run_curl "[encoding] Deflate compressed JSON (httpbin.org/deflate)" \
    -H "Accept-Encoding: deflate" "https://httpbin.org/deflate"
}

# 9. Slow request (>= 1s elapsed warning color & slow: filter)
scenario_slow() {
  run_curl "[slow] Delayed response (~1.3s, triggers warning color & slow: filter)" \
    "https://dummyjson.com/products/search?q=phone&limit=3&delay=1300"
}

# 10. Status codes (204 No Content, 302 Redirect, 404 Not Found, 502 Bad Gateway)
scenario_status() {
  run_curl "[status] 204 No Content (empty body completion)" \
    "https://httpbin.org/status/204"
  run_curl "[status] 302 Redirect" \
    "https://httpbin.org/status/302"
  run_curl "[status] 404 Not Found" \
    "https://httpbin.org/status/404"
  run_curl "[status] 502 Bad Gateway" \
    "https://httpbin.org/status/502"
}

# 11. Server-Sent Events (text/event-stream in Messages card)
scenario_sse() {
  run_curl "[sse] Server-Sent Events stream (postman-echo)" \
    "https://postman-echo.com/stream-events/3" \
    -H "Accept: text/event-stream"
}

# 12. Origin TLS handshake failure (badssl.com, visible when 'Tunnel failures' is enabled)
scenario_tls() {
  run_curl "[tls] Expired origin certificate (expired.badssl.com)" \
    "https://expired.badssl.com/"
  run_curl "[tls] Self-signed origin certificate (self-signed.badssl.com)" \
    "https://self-signed.badssl.com/"
}

case "$GROUP" in
  local)    scenario_local ;;
  json)     scenario_json ;;
  methods)  scenario_methods ;;
  form)     scenario_form ;;
  cookie)   scenario_cookie ;;
  image)    scenario_image ;;
  hex)      scenario_hex ;;
  encoding|gzip) scenario_encoding ;;
  slow)     scenario_slow ;;
  status)   scenario_status ;;
  sse)      scenario_sse ;;
  tls)      scenario_tls ;;
  all)
    scenario_local
    scenario_json
    scenario_methods
    scenario_form
    scenario_cookie
    scenario_image
    scenario_hex
    scenario_encoding
    scenario_slow
    scenario_status
    scenario_sse
    scenario_tls
    ;;
  *)
    echo "Unknown scenario group: $GROUP" >&2
    echo "Available groups: all | local | json | methods | form | cookie | image | hex | encoding | slow | status | sse | tls" >&2
    exit 1
    ;;
esac
