# zniff Rules & Configuration Guide

English | [简体中文](./rules-zh.md) | [Docs Index](./README.md)

This guide covers where **zniff** stores configuration and certificates, how to use the traffic filter syntax, TLS bypass lists, proxy allowed-host restrictions (`allow_hosts`), all 17 rule types (routing, rewrites, breakpoints, host-level TLS/DNS, local HTTP webhooks, and `.proto` schema decoding), dynamic template variables with `HMAC-SHA256` signing, upstream proxy chaining, external root CA reuse, machine-wide settings, and saved setups (`.toml` presets).

---

## Contents

1. [Configuration & Data Directory](#1-configuration--data-directory)
2. [Traffic Filter Syntax](#2-traffic-filter-syntax)
3. [TLS Bypass List (No MITM)](#3-tls-bypass-list-no-mitm)
4. [Traffic Rules Reference (17 Rule Types)](#4-traffic-rules-reference-17-rule-types)
   - [4.1 General Syntax & `<pattern>` Matching](#41-general-syntax--pattern-matching)
   - [4.2 Rule Categories & Quick Reference](#42-rule-categories--quick-reference)
   - [4.3 Terminal Rules](#43-terminal-rules)
   - [4.4 Modifier Rules](#44-modifier-rules)
   - [4.5 Host-Level Rules (TLS / DNS)](#45-host-level-rules-tls--dns)
   - [4.6 Extension & Schema Rules (Webhook / Protobuf)](#46-extension--schema-rules-webhook--protobuf)
   - [4.7 Dynamic Template Variables & Signing (`{{...}}`)](#47-dynamic-template-variables--signing)
   - [4.8 GraphQL Operation Matching (`op=`)](#48-graphql-operation-matching-op)
   - [4.9 Multi-Group Rules (`[[rule_groups]]`)](#49-multi-group-rules-rule_groups)
   - [4.10 Migrating from Whistle & Charles](#410-migrating-from-whistle--charles)
5. [Global Switches, Upstream Proxy & Machine Settings](#5-global-switches-upstream-proxy--machine-settings)
6. [Saved Setups (`SavedProfile`) & TOML Import/Export](#6-saved-setups-savedprofile--toml-importexport)

---

## 1. Configuration & Data Directory

All persistent settings and the local Root CA are stored in your operating system's user data directory (resolved by [`ca::data_dir()`](../app/src/ca.rs)):

| Operating System | Data Directory Path |
| :--- | :--- |
| **macOS** | `~/Library/Application Support/zniff` |
| **Windows** | `%APPDATA%\zniff` (e.g., `C:\Users\<username>\AppData\Roaming\zniff`) |
| **Linux** | `$XDG_DATA_HOME/zniff` (falls back to `~/.local/share/zniff`) |

Override the directory with `--data-dir <dir>` or the `ZNIFF_DATA_DIR` environment variable (the Docker image defaults to `ZNIFF_DATA_DIR=/data`).

### Files in the Data Directory

| File Name | Purpose |
| :--- | :--- |
| `config.toml` | **Production configuration** (default port `8877`): stores listener settings, window size/position/display/mode, layout & column widths, primary filter, TLS bypass list, rules, upstream proxy, and saved setups (`saved_profiles`). |
| `proxy-snapshot.toml` | **Production system-proxy recovery snapshot**: backs up original OS proxy settings before applying the system proxy; restored automatically on next launch after an unexpected exit. |
| `config.dev.toml` | **Development configuration** (`make dev`, debug builds, or `ZNIFF_DEV=1`, default port `8878`): isolated from `config.toml` so dev runs never overwrite production settings. |
| `proxy-snapshot.dev.toml` | **Development system-proxy recovery snapshot**. |
| `ca.crt` / `ca.key` | Local `zniff CA` root certificate and private key (shared across production and dev modes; optionally overridden by reusing a trusted local external CA from Whistle, mitmproxy, mkcert, or Proxyman via `external_ca`). |
| `certs/` (optional) | Directory for auto-loaded custom server certificates keyed by hostname (including `_wildcard.<domain>`). |

> **Temporary Large-Body Spill Store**: Completed unpinned request/response bodies larger than `16 KB` are appended to a rotating ring buffer in the system temp directory (e.g., `$TMPDIR/zniff-spill/`, 64 MB per segment, 512 MB cap) and cleaned up on exit or after 30 minutes idle.

---

## 2. Traffic Filter Syntax

`zniff` uses a two-layer filter model:
- **Primary Filter (Persisted)**: Configured on the left side of the top **Filters** popover (structured **Include Domains / Include Path Prefixes / Exclude Host or Path** fields and quick-toggle chips on top, synced bi-directionally with the raw expression input below). Persisted in `config.toml` (`filter`) and saved with setups.
- **Secondary List Filter (Session-only)**: The collapsible domain/app sidebar (default on in `Split` layout) and the search bar at the top of the flow list narrow results *within* the active primary filter. When non-empty, a **Merge** button appears next to the search box to fold those terms into the primary filter and save.

Filters use **space-separated `AND` tokens**, with `|` or `,` inside a token for **`OR`**:

| Token / Prefix | Description | Example |
| :--- | :--- | :--- |
| `host:<domain>` or `domain:<domain>` | **Exact** (case-insensitive), subdomain wildcard (`*.example.com`), or `*` glob host match; supports `\|` / `,` for `OR` | `host:api.example.com\|*.cdn.com` |
| `path:<prefix>` | Match request path prefix (leading `/` optional), supports `\|` / `,` for `OR` | `path:/api/v1\|/auth\|/graphql` |
| `ip:<addr>` / `client:<addr>` / `app:<name>` / `proc:<name>` | Match client source IP or local process name, supports `\|` / `,` for `OR` | `ip:127.0.0.1\|192.168.1.25`, `app:curl\|node` |
| `GET` / `POST` / `PUT` / `DELETE` / `CONNECT` … or `method:<m>` | Exact HTTP method match (case-insensitive), supports `\|` for `OR` | `POST host:api.example.com`, `method:put\|patch` |
| `200` / `404` / `1xx`–`5xx` or `status:<code>` | Exact status code, status class (`1xx`–`5xx`), or `err` | `5xx`, `404`, `status:401\|403\|err` |
| `fail` / `error` / `err` / `失败` | Match failed flows (`ERR` or status `>= 400`) | `fail` |
| `type:<kind>` or `ct:<kind>` | Match response `Content-Type` substring | `type:json`, `ct:html`, `type:image` |
| `body:<keyword>` | Search decompressed request/response bodies and WebSocket/SSE messages (including disk-spilled bodies) | `body:access_token` |
| `header:<keyword>` | Search request and response header names and values | `header:authorization`, `header:x-trace-id` |
| `note:<keyword>` or `comment:<keyword>` | Match flow notes by substring (bare `note:` matches any annotated flow) | `note:bug`, `note:` |
| `repeat:` or `repeat:<id>` | Match batch-replayed flows and their source flow; `repeat:<id>` narrows to one source | `repeat:`, `repeat:42` |
| `op:<name>` / `gql:<name>` / `operation:<name>` | Match GraphQL operation names (explicit `operationName`, inferred named operations, `anonymous`, batched arrays); supports `*` and `\|` | `op:GetUser*\|Checkout` |
| `pin:` | Show only starred flows | `pin:` |
| `slow:` | Show only slow flows (elapsed time `>= 1s`) | `slow:` |
| `mute:<term>` or `-<term>` | Exclude flows whose host or path contains `<term>`, supports `\|` for `OR` | `mute:ads.example.com\|/health` or `-telemetry` |
| `-host:` / `-path:` / `-status:` / `-method:` / `-type:` / `-app:` / `-header:` / `-body:` / `-note:` / `-op:` / `-repeat:` / `-pin:` / `-slow:` | Negated form of any structured token above | `-host:*.cdn.com -status:3xx -pin:` |
| Bare word | Case-insensitive **substring** match against host, path, or flow note | `login` |

> **Note**: `host:` / `domain:` matches exact hostnames or wildcards (so `host:api.example.com` does not accidentally match `sub.api.example.com`; use `*.example.com` for subdomains). For substring host matching, use a bare word.

**Combined Filter Example:**

```text
host:api.example.com|*.staging.test path:/v1|/v2 POST type:json mute:/health
```

Matches `POST` requests to `api.example.com` or `*.staging.test` whose path starts with `/v1` or `/v2` and whose response is JSON, excluding `/health`.

---

## 3. TLS Bypass List (No MITM)

Configured in **TLS Bypass (No MITM)** on the right side of the **Filters** popover (`bypass_hosts` in `config.toml`).
Evaluated against the TLS `ClientHello` SNI (or the `CONNECT` host if SNI is absent):

- **Empty list (default)**: Decrypts (MITM) all hosts.
- **Denylist mode (default)**: Listed hosts skip TLS interception and become transparent TCP tunnels (recorded as `Bypass list, not decrypted`). Ideal for clients using certificate pinning.
- **Allowlist mode (`only:` prefix)**: Only decrypts the listed hosts; every other host is tunneled transparently.
- **Separators & wildcards**: Separate entries with spaces, commas `,`, semicolons `;`, or pipes `|`. Supports exact hosts, `*.example.com` (subdomains only, not apex `example.com`), and `*` (all hosts).
- **Dynamic `/proxy.pac` endpoint**: The capture port serves `GET/HEAD /proxy.pac` (and `/zniff.pac`) reflecting both `allow_hosts` (disallowed hosts or strict deny-all always receive `DIRECT`) and `bypass_hosts` (both denylist and `only:` modes) while sending loopback addresses `DIRECT` (copyable from the footer **Connect** popover).

```text
# Example 1: Skip TLS interception for Apple services and a pinned host
*.apple.com, *.icloud.com | pin.example.com

# Example 2: Allowlist mode — only decrypt api.example.com and *.dev.internal
only: api.example.com *.dev.internal
```

---

## 4. Traffic Rules Reference (17 Rule Types)

Configured in **Rules** on the right side of the **Filters** popover (`rules` in `config.toml`).

- **Live Validation & Instant Reload**: The editor shows 1-based line numbers and highlights syntax-error line numbers in real time. Changes apply immediately to new requests without restarting the proxy.
- **Per-Line Rule Assist Bar**: Below the editor, each rule line can be **commented/uncommented (`#`)**, **loaded back into the Quick Add form for editing**, or **deleted** with one click.
- **16 Quick Add Tabs**: Covers all common rules (every rule except `proto`), with one-click `{{ts_ms}}` / `{{uuid}}` / `{{hmac_sha256(...)}}` variable insertion chips on value fields that support templates.

### 4.1 General Syntax & `<pattern>` Matching

- **One rule per line**. Anything after `#` is treated as a comment.
- The second token is always `<pattern>` (**no spaces allowed**), written as `<host-pattern>[/<path-prefix>]`:
  1. **Host part**: Exact host (`api.example.com`), wildcard subdomain (`*.example.com`, excludes apex `example.com`), all hosts (`*`), or multiple hosts joined with `|` / `,` (`api.example.com|*.staging.example.com`).
  2. **Path prefix part** (optional; omitting it or `/` matches all paths): Segment-boundary prefix match ignoring `?query` (`/api` matches `/api`, `/api/`, `/api/v1`, but **not** `/apix`). Join multiple prefixes after the first `/` with `|` / `,` (`example.com/api|/v2|/graphql`), or match any host with `*/api|/v2`.
- Wrap arguments containing spaces (or starting with `"`) in double quotes `"..."` (escape inner quotes as `\"`).

### 4.2 Rule Categories & Quick Reference

| Category | Keyword (Aliases) | Precedence | Supports `op=` | Supports `{{vars}}` | Purpose |
| :--- | :--- | :--- | :---: | :---: | :--- |
| **Terminal** | `map-local` | First match | ✅ | — | Serve a local file/directory (with optional `fallback`) or status-only empty response |
| **Terminal** | `map-remote` | First match | ✅ | — | Forward to another HTTP/HTTPS origin (rewrites `Host`) |
| **Terminal** | `break` | First match | ✅ | — | Pause requests before forwarding; edit headers/body, continue, or drop |
| **Terminal** | `break-res` (`break-response`) | First match | ✅ | — | Pause responses before returning; edit status/headers/body |
| **Terminal** | `block` | First match | ✅ | — | Return synthetic status code (default `403`) or drop/reset connection |
| **Modifier** | `replace` | Additive | ✅ | ✅ | Search-and-replace in request URL/body or response body (incl. `<= 1 MB` chunked) |
| **Modifier** | `json` | Additive | ✅ | ✅ | Set/delete JSON fields by JSON Pointer or dot/bracket path (preserves key order) |
| **Modifier** | `header` | Additive | ✅ | ✅ | Set or delete request/response headers |
| **Modifier** | `query` | Additive | ✅ | ✅ | Set or delete URL query parameters |
| **Modifier** | `delay` | Additive | ✅ | — | Inject request latency (cumulative cap `30s`) |
| **Modifier** | `throttle` | Additive | — | — | 4 KB chunked bandwidth throttling (`2g`/`3g`/`4g` presets or custom rates) |
| **Modifier** | `cors` | Additive | — | — | Inject CORS headers and answer `OPTIONS` preflight with `204` when non-terminal |
| **Modifier** | `no-cache` (`nocache`) | Additive | ✅ | — | Strip conditional headers & force `no-cache`/`no-store` on both sides to prevent `304` |
| **Host-level** | `cert` / `client-cert` (`mtls`) | Host only | — | — | Mount custom MITM server cert or upstream mTLS client cert |
| **Host-level** | `resolve` (`dns` / `hosts`) | Host only | — | — | Override dial IP/host and optional port while preserving `Host` and TLS SNI |
| **Extension** | `hook` (`webhook`) | Additive | ✅ | — | Call loopback HTTP service to mutate payloads (`mutate`) or decrypt in detail view (`preview`) |
| **Schema** | `proto` | Read-only | — | — | Mount `.proto` file or directory for Protobuf/gRPC schema decoding in detail view |

> **Precedence**: **Terminal rules** use first-match-wins in top-to-bottom order. **Modifier rules** never consume the terminal slot—every matching modifier stacks in order, even if written below the winning terminal rule.

---

### 4.3 Terminal Rules

#### ① `map-local` — Local File, Directory, or Status-Only Mock Response

Answers directly from disk (or an empty body when only `status=` is given) **without dialing the origin**. `Content-Type` is inferred from the file extension.

When targeting a **directory**, `zniff` strips the matched path prefix and `?query`, percent-decodes segments, and resolves files safely inside that root: directories check `index.html` then `index.json`; extensionless paths check the exact filename then `<segment>.json` (compatible with directories generated by **Generate Local Mock…**). Path traversal outside the root is rejected.

```text
map-local <pattern> [<local-file-or-dir>] [status=<100..599>] [delay=<duration>] [fallback] [op=<GraphQL-Operation>]
```

> `status=` defaults to `200`; omitting the file requires `status=`. Without `fallback`, missing files return `502`; with `fallback`, missing files fall back to the origin (noted `map-local(fallback)`). Right-click any captured flow → **Map local** to save its response and append a rule automatically.

```text
map-local api.example.com/v1/profile ./mocks/profile.json
map-local api.example.com/v1/profile "/Users/alice/My Mocks/profile.json" status=200
map-local api.example.com/v1/maintenance status=503
map-local api.example.com/v1/orders /tmp/rate_limit.json status=429 delay=1500ms
map-local www.example.com/static ./dist fallback
```

#### ② `map-remote` — Forward to Another Origin / URL

Redirects matching requests to another `http://` or `https://` target, rewriting `Host` while keeping remaining path segments and query parameters.

```text
map-remote <pattern> <http://-or-https://-target-url> [op=<GraphQL-Operation>]
```

Example path join: `<pattern>` = `example.com/api`, target = `https://staging.internal:8443/v2` → `https://example.com/api/users?id=1` forwards to `https://staging.internal:8443/v2/users?id=1`.

```text
map-remote api.example.com/api http://127.0.0.1:3000/api
map-remote test.example.com https://staging-api.example.com
```

#### ③ `break` — Request Breakpoint

Pauses matching requests before forwarding to the origin; a yellow breakpoint bar appears at the top:
- Click **Edit request** (method and URL are read-only) to modify **headers and body**, then click **Continue** or press `Cmd/Ctrl+Enter`.
- Click **Drop** to close the client connection/stream immediately (recorded as `ERR · Breakpoint dropped`); click **Continue all** to release all queued items.
- Request and response breakpoints share an **8-slot bounded FIFO queue**: when multiple flows are queued, `#id request/response` chips appear—click any chip to move it to the front and select it. The front item auto-releases after 60s of inactivity (extended while editing).

```text
break <pattern> [op=<GraphQL-Operation>]
```

```text
break api.example.com/v1/checkout
break */auth/login
```

#### ④ `break-res` (`break-response`) — Response Breakpoint

Forwards to the origin, buffers up to `1 MB` of the response, and pauses before sending it back so you can edit **status code, response headers, and response body**.

```text
break-res <pattern> [op=<GraphQL-Operation>]
```

```text
break-res api.example.com/v1/permissions
```

#### ⑤ `block` — Block Request or Drop Connection

Stops matching requests at the proxy without dialing the origin: returns a synthetic empty response (default `403`) or immediately closes/resets the connection with `drop` / `reset` / `abort`.

```text
block <pattern> [status=<100..599>|drop|reset|abort] [op=<GraphQL-Operation>]
```

```text
block telemetry.example.com status=403
block ads.example.com drop
```

---

### 4.4 Modifier Rules

#### ⑥ `replace` — Text Search & Replace

Applies sequential UTF-8 replacements (supports both `Content-Length <= 1 MB` and `Transfer-Encoding: chunked <= 1 MB`, decompressing and recalculating `Content-Length`):
- **`req`**: Rewrites both request target (`/path?query`, result must still start with `/`) and request body.
- **`res`**: Rewrites response body.
- Leave the right-hand side of `->` empty to delete matches. Supports `{{...}}` variables and `op=`.

```text
replace <pattern> req|res <find> -> [<replacement>] [op=<GraphQL-Operation>]
```

```text
replace api.example.com/v1 res "\"env\":\"prod\"" -> "\"env\":\"debug\""
replace api.example.com/articles req draft=1 -> draft=0
replace api.example.com/api req ?debug=true ->
```

#### ⑦ `json` — JSON Field Patch / Delete

Mutates JSON request or response bodies (`<= 1 MB`, including `chunked`) in memory while preserving original object key order. Supports JSON Pointer (`/a/b/0`) or dot/bracket paths (`a.b[0]`), valid JSON literals, and `{{...}}` variables.

```text
json <pattern> req|res set <path> <json-value> [op=<GraphQL-Operation>]
json <pattern> req|res del <path> [op=<GraphQL-Operation>]
```

```text
json api.example.com/v1/profile res set /data/is_vip true
json api.example.com/v1/items res set items[0].price 0
json api.example.com/events req del /metadata/device_fingerprint
```

#### ⑧ `header` — Set or Delete Headers

Sets (`set` / `add`, case-insensitive replace) or deletes (`del` / `delete` / `rm`) request (`req`) or response (`res`) headers. Values may contain spaces and `{{...}}` variables.

```text
header <pattern> req|res set <Header-Name> <Header-Value> [op=<GraphQL-Operation>]
header <pattern> req|res del <Header-Name> [op=<GraphQL-Operation>]
```

```text
header api.example.com req set X-Debug-Env staging-gray
header api.example.com/public req del Cookie
header *.example.com res del Content-Security-Policy
header *.example.com res set Cache-Control no-store
```

#### ⑨ `query` — Set or Delete URL Query Parameters

Rewrites URL query parameters before forwarding (`set`/`add` or `del`/`delete`/`rm`), handling separators and percent-encoding automatically. Supports `{{...}}` variables.

```text
query <pattern> set <key> <value> [op=<GraphQL-Operation>]
query <pattern> del <key> [op=<GraphQL-Operation>]
```

```text
query api.example.com/v1 set env staging
query *.example.com del utm_source
```

#### ⑩ `delay` — Latency Injection

Waits before forwarding; matching delays accumulate up to a **30-second (`30s` / `30000ms`) cap**.

```text
delay <pattern> <duration(ms-or-s)> [op=<GraphQL-Operation>]
```

```text
delay api.example.com/v1/feed 2000ms
delay api.example.com/v1/upload 5s
```

#### ⑪ `throttle` — Bandwidth Throttling

Throttles body streaming in 4 KB chunks (bandwidth only; stack with `delay` to add RTT):
- Presets (down / up): `2g` / `slow-3g` (~400 / 160 kbps), `3g` / `fast-3g` (~768 / 256 kbps), `4g` / `slow-4g` (~4 / 1 Mbps).
- A single rate applies symmetrically; use `down=` / `dl=` and `up=` / `ul=` for asymmetric limits.
- Units (case-insensitive): `kbps` / `kbit/s` / `kb` / bare number = kilobits/s; `mbps` / `mbit/s` / `mb` = megabits/s; `kb/s` / `kbyte/s` = KiB/s; `mb/s` / `mbyte/s` = MiB/s; `b/s` / `bps` = bytes/s. Bare `k` (e.g., `500k`) is not accepted.

```text
throttle <pattern> <preset|rate|down=<rate> [up=<rate>]>
```

```text
throttle api.example.com/mobile 3g
throttle cdn.example.com/video down=500kb/s
throttle api.example.com/upload down=2mbps up=512kbps
```

#### ⑫ `cors` — Automatic CORS & Preflight

Fills in missing CORS response headers (`Access-Control-Allow-Origin: *`, `Access-Control-Allow-Methods: GET, POST, PUT, PATCH, DELETE, OPTIONS, HEAD`, `Access-Control-Allow-Headers: *`, `Access-Control-Max-Age: 86400`). For browser `OPTIONS` preflights that match no terminal rule, responds directly with `204 No Content` + CORS headers.

```text
cors <pattern>
```

```text
cors api.example.com
cors *
```

#### ⑬ `no-cache` (`nocache`) — Disable HTTP Caching

Bypasses both client and origin caches to eliminate `304 Not Modified`:
- **Request**: Strips `If-Modified-Since` and `If-None-Match`; sets `Cache-Control: no-cache, no-store, must-revalidate` and `Pragma: no-cache`.
- **Response**: Strips `ETag` and `Last-Modified`; sets `Cache-Control: no-cache, no-store, must-revalidate`, `Pragma: no-cache`, and `Expires: 0`.

```text
no-cache <pattern> [op=<GraphQL-Operation>]
```

```text
no-cache static.example.com/assets
```

---

### 4.5 Host-Level Rules (TLS / DNS)

> These rules match by **host only** and do not support `op=`. `resolve` does not allow a path or port inside `<pattern>`.

#### ⑭ `cert` / `client-cert` (`mtls`) — Custom Server Cert or Upstream mTLS

`<key-path>` may be omitted when the PEM file contains both the certificate chain and private key:
- **`cert`**: Present your own server certificate to clients instead of a leaf signed by the local Root CA.
- **`client-cert`** (alias `mtls`): Present a client certificate to origins requiring mutual TLS (displayed in connection metadata).

```text
cert        <host-pattern> <cert-path> [<key-path>]
client-cert <host-pattern> <cert-path> [<key-path>]
```

```text
cert *.internal.corp /etc/ssl/internal.crt /etc/ssl/internal.key
client-cert secure.bank.com ./certs/client.pem ./certs/client.key
mtls partner.example.com ./certs/partner-bundle.pem
```

#### ⑮ `resolve` (`dns` / `hosts`) — Custom Host / IP Resolution

Routes TCP dials to `<target-ip-or-host>[:port]` while **preserving the original HTTP `Host` header, TLS SNI, and server certificate hostname verification**.

```text
resolve <host-pattern> <target-ip-or-host[:port]> [<port>]
```

```text
resolve api.example.com 127.0.0.1
resolve *.staging.example.com 10.0.0.12:8443
dns api.example.com [::1]:8080
```

---

### 4.6 Extension & Schema Rules (Webhook / Protobuf)

#### ⑯ `hook` (`webhook`) — Loopback HTTP Webhook Extension

Delegate custom encryption/decryption, signing, or complex payload transforms to a local HTTP server in any language (Python, Node, Go, Rust) without embedding a script VM in `zniff`.

**Guardrails**:
- Restricted strictly to loopback HTTP URLs (`http://127.0.0.1:...`, `http://localhost:...`, `http://[::1]:...`) to prevent accidental exfiltration.
- Connects directly (bypassing the `zniff` proxy and flow list), bounded by `timeout` (default `2000ms`, range `50ms..=30s`) and `1 MB` payload cap; fails open with a flow note on error/timeout.

```text
hook <pattern> <http://127.0.0.1:port/endpoint> [req|res|both] [mode=mutate|preview] [timeout=2000ms] [op=<GraphQL-Operation>]
```

Two modes and JSON wire formats:

1. **`mode=mutate` (default) — In-flight mutation during forwarding**
   - `zniff` POSTs JSON:
     - Request phase: `{"id", "phase": "request", "method", "url", "host", "path", "headers": [["Name","Val"], ...], "body_base64", "body_utf8"}`
     - Response phase: `{"id", "phase": "response", "method", "url", "status", "headers": [["Name","Val"], ...], "body_base64", "body_utf8"}`
   - Hook responds with `2xx` JSON (all fields optional; omitted fields stay unchanged):
     - `path` (request only, must start with `/`), `status` (response only, `100..=599`), `headers` (`[["K","V"]]` or `{"K":"V"}`), `body_base64` or `body_utf8`, `comment` (appended to flow note).

2. **`mode=preview` — Read-only detail-view decryption (zero wire modification)**
   - Forwarded bytes are untouched. When viewing Pretty / Tree mode in the detail pane, `zniff` asynchronously invokes the hook and caches the decoded output; switching to Raw shows the original ciphertext.
   - `zniff` POSTs: `{"id", "phase": "req"|"res", "mode": "preview", "method", "url", "status", "headers", "body_b64"}`
   - Hook responds with: `{"body_b64" | "body" (string or JSON object/array), "content_type"?, "note"? | "comment"?}`

```text
# Mutate request/response payloads through a local Python/Node service
hook api.example.com/v1/pay http://127.0.0.1:9001/mutate both timeout=3000ms

# Read-only decryption in detail view without altering wire traffic
hook api.example.com/v1/cipher http://127.0.0.1:9001/decrypt res mode=preview
```

#### ⑰ `proto` — Zero-Dependency `.proto` Schema Decoding

Mounts a single `.proto` file or directory of `.proto` files (parsed in pure Rust with **zero external dependencies and no `protoc` required**) to upgrade Protobuf/gRPC detail views from schema-less `field #N` wire tags to named JSON fields, enums, nested messages, `oneof`, `map<K,V>`, and interactive JSON Tree View.

```text
proto <pattern> <proto-file-or-dir> [req=<pkg.ReqMessage>] [res=<pkg.ResMessage>]
```

- For standard gRPC `:path` (`/package.Service/Method`), omitting `req=` / `res=` automatically resolves input and output message types from the `service` definition.
- For plain HTTP Protobuf endpoints, specify `req=` and/or `res=` explicitly.

```text
proto grpc.example.com ./protos
proto api.example.com/v1/order ./protos/order.proto req=shop.CreateOrderReq res=shop.CreateOrderRes
```

---

### 4.7 Dynamic Template Variables & Signing (`{{...}}`)

`header`, `query`, `replace`, and `json` rules evaluate `{{...}}` expressions at proxy time (supports `+` concatenation; unrecognized `{{...}}` expressions are left untouched):

| Category | Variables / Functions | Description |
| :--- | :--- | :--- |
| **Time & Random** | `{{uuid}}`, `{{ts}}`, `{{ts_ms}}`, `{{iso_now}}`, `{{random_int(min,max)}}` | UUID v4, Unix seconds/milliseconds, UTC ISO-8601 timestamp, random integer in range |
| **Request Context** | `{{req.method}}`, `{{req.host}}`, `{{req.path}}`, `{{req.target}}`, `{{req.query.<key>}}`, `{{req.header.<name>}}`, `{{req.body}}` | Current request method, host, path, path+query target, query value, header value, and body |
| **Response Context** | `{{res.status}}`, `{{res.header.<name>}}`, `{{res.body}}` | Available in response-side rules (`header ... res`, `replace ... res`, `json ... res`) |
| **Digests & Encoding** | `{{sha256(x)}}`, `{{sha256_b64(x)}}`, `{{hmac_sha256(secret, msg)}}`, `{{hmac_sha256_b64(secret, msg)}}`, `{{base64(x)}}`, `{{urlencode(x)}}` | Pure-Rust SHA-256 / HMAC-SHA256 (hex & Base64), Base64, and URL encoding |

> When a request matches both a `json`/`replace` body rewrite and a `header` rule referencing `req.body`, `zniff` re-evaluates the header after the body rewrite so signatures always match the final transmitted payload.

```text
header api.example.com/v1 req set X-Request-Id {{uuid}}
header api.example.com/v1 req set X-Timestamp {{ts_ms}}
header api.example.com/v1 req set X-Signature {{hmac_sha256("dev-secret", req.method + "\n" + req.path + "\n" + req.body)}}
```

---

### 4.8 GraphQL Operation Matching (`op=`)

**12 rule types** support appending `op=<name>` (or `operation=<name>`):
`map-local`, `map-remote`, `break`, `break-res`, `block`, `replace`, `json`, `delay`, `header`, `query`, `no-cache`, and `hook`.
(Not supported on: `cors`, `throttle`, `cert`/`client-cert`, `resolve`, `proto`.)

- Matches explicit `operationName`, inferred named queries/mutations/subscriptions, `anonymous`, and batched arrays (bounded `<= 1 MB` request reading on both HTTP and HTTPS).
- Supports `*` wildcards and `|` OR unions.

```text
map-local api.example.com/graphql ./mocks/user.json op=GetUserInfo
delay     api.example.com/graphql 3000ms op=Submit*
break     api.example.com/graphql op=Checkout|Payment
```

### 4.9 Multi-Group Rules (`[[rule_groups]]`)

Alongside the workspace `rules` buffer, `config.toml` supports up to `32` independently toggleable named groups under `[[rule_groups]]` (synchronized with `--web` and MCP `get_rules` / `set_rules`):

- **Evaluation order**: Workspace `rules` evaluate first (so ad-hoc right-click `break` / `map-local` edits override background groups), followed by each enabled `RuleGroup` in order. First matching terminal rule wins; modifier rules accumulate.
- **UI**: Use the compact pill bar above the rules editor in **Filters** to toggle checkboxes `[✓/☐]`, switch which buffer you are editing, delete groups, or create new ones via **`[+ Group]`**.

```toml
[[rule_groups]]
name = "Local Dev"
enabled = true
rules = """
resolve api.example.com 127.0.0.1:8080
cors api.example.com
"""

[[rule_groups]]
name = "Throttle 3G"
enabled = false
rules = "throttle api.example.com 3g"
```

### 4.10 Migrating from Whistle & Charles

- **Whistle `.txt`**: Drag and drop a `.txt` rules file or paste Whistle syntax into **Filters → Import** (handles host/IP mapping, `http(s)://`, `file://`/`xfile://`, header/query/delay/cors/status/nocache/speed/replace, `ignore://`, and fenced `{block}` values; unconvertible lines stay commented as `# [unmapped whistle] ...`).
- **Charles `.xml` & `.chlsj`**: Import Charles XML exports (Map Local/Remote, Rewrite, Breakpoints, DNS Spoofing, No Caching, Throttling) or `.chlsj` JSON session captures.
- Imported rule sets automatically upsert an enabled named `RuleGroup` and merge bypass hosts without overwriting workspace `rules`.

---

## 5. Global Switches, Upstream Proxy & Machine Settings

### 5.1 Global Switches & Upstream Proxy (Saved in Setups)

At the top and bottom of the right column in the **Filters** popover:

1. **Rules (`启用规则`)**: Temporarily disables all traffic rules without erasing your rules or editing `config.toml` (the toolbar **Filters** button turns amber).
2. **Breaks (`启用断点`)**: Temporarily skips `break` and `break-res` pauses while keeping all other rules active.
3. **Tunnel errors (`隧道失败`)**: Off by default. Enable to record origin TLS probe failures that fell back to TCP tunneling in the flow list.
4. **Upstream Proxy (`upstream_proxy`)**: Chain to an upstream HTTP/CONNECT proxy, `socks5://` (local DNS, default port `1080`), or `socks5h://` (remote DNS resolution via SOCKS5, default port `1080`, RFC 1929 auth supported), or leave empty for direct origin connections:
   ```text
   127.0.0.1:7890
   http://proxy.internal.corp:8080
   http://alice:p%40ss@proxy.internal.corp:8080
   socks5://127.0.0.1:1080
   socks5h://user:p%40ss@127.0.0.1:1080
   ```

### 5.2 Machine Settings (Outside Saved Setups)

These settings belong to this machine rather than any setup. They save immediately to `config.toml` and are **never** included in saved setups or exports:

| Setting | UI Location | `config.toml` key | Description |
| :--- | :--- | :--- | :--- |
| **Proxy auth** | Footer **Access** popover | `proxy_auth` | `user:pass`. Requires HTTP Basic `Proxy-Authorization` on the capture port (`407` on failure); stripped before forwarding. `/zniff`, `/zniff-ca.crt`, `/proxy.pac`, `/zniff.pac`, `/s/{uuid}`, `/r/{uuid}`, and loopback `/mcp` are exempt. Overridden by `--proxy-auth` / `ZNIFF_PROXY_AUTH`. |
| **Allowed hosts** | Footer **Access** popover | `allow_hosts` | Restricts proxied domains. Supports exact host/IP/`[::1]`, `*.sub`, `*` (allow-all), `-host` exclusions (always win), and `#` comments. Precedence: `--allow-hosts` > non-empty `ZNIFF_ALLOW_HOSTS` > `config.toml` (CLI/env locks UI & `/api/hub` read-only with `403`). Non-strict empty allows all; strict mode (`--strict-allow-hosts` / `ZNIFF_STRICT_ALLOW_HOSTS=1`, default in Docker) denies all when empty (`403 Forbidden`, not recorded). Enforced on `CONNECT`/SNI, HTTP/HTTPS/H2, `map-remote`, replay, and `/proxy.pac`; local routes (`/zniff`, `/zniff-ca.crt`, `/proxy.pac`, `/zniff.pac`, `/mcp`, `/s/`, `/relay`) are exempt. |
| **External root CA** | Top **More → Certificate** / Footer **Connect** | `external_ca` | Optional trusted local external root CA provider (`"whistle"`, `"mitmproxy"`, `"mkcert"`, `"proxyman"`); empty uses built-in `zniff CA`. |
| **Relay URL** | Top **More → Relay Settings** | `relay_url` | Base URL of a `zniff --relay` server for live sharing (empty = local-only sharing). |
| **Relay token** | Top **More → Relay Settings** | `relay_token` | Bearer token for uploading live sessions to the relay. |
| **Accept relay** | Top **More → Relay Settings** | `relay_accept` | Off by default; enables receiving relayed sessions into this instance's flow list. |

---

## 6. Saved Setups (`SavedProfile`) & TOML Import/Export

The bottom bar of the **Filters** popover bundles **Primary Filter + TLS Bypass + Rules + Upstream Proxy + 3 Switches** into a named setup (up to 16 setups in `config.toml` under `[[saved_profiles]]`):

- **Switching & Toolbar Status Pill**: Selecting a setup populates all fields immediately (`active_profile`). The toolbar displays an integrated `[✓ <name / filter>]` status pill:
  - When a primary filter is non-empty, clicking the pill **pauses or resumes the primary filter** (without affecting rules or secondary list filters).
  - When the primary filter is empty (e.g., a rule-only setup), clicking the pill opens the **Filters** popover directly while the summary previews rule count (e.g., `Rules×3: ...`).
  - To deselect the active setup without deleting it, choose **Deselect current setup** at the top of the setup dropdown (or click the active entry again).
- **History**: The history dropdown retains the 10 most recent setup snapshots (`profile_history`) for quick rollback.
- **Save & New**: Enter a name and click **Save** (any filled Quick Add draft is automatically appended; matching names overwrite case-insensitively; disabled when unchanged). **New** resets the workspace and switches without deleting saved setups.
- **Export & Import `.toml`**:
  - **Export** serializes setups as `.toml` text and copies to the clipboard.
  - **Import** opens a multiline textarea to paste `.toml` setups, or **drag and drop a `.toml` file directly onto the `zniff` window** to merge by name.

```toml
[[profiles]]
name = "Local API Dev"
filter = "host:api.example.com|127.0.0.1 mute:/health"
bypass_hosts = "*.apple.com, *.icloud.com"
rules = """
# Local API routing, CORS, and dynamic timestamp header
cors api.example.com
header api.example.com req set X-Env local-dev
header api.example.com req set X-Timestamp {{ts_ms}}
map-remote api.example.com/v1 http://127.0.0.1:3000/v1
map-local api.example.com/v2/feature-flags ./mocks/flags.json
"""
upstream_proxy = ""
```
