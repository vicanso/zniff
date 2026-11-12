# zniff Rules & Configuration Guide

English | [简体中文](./rules-zh.md)

This guide covers where **zniff** stores its configuration files, how to use the traffic filter syntax, TLS bypass lists, all 14 traffic modification & breakpoint rules, GraphQL operation matching, upstream proxy chaining, and saved setups (`.toml` presets).

---

## 1. Configuration & Data Directory

All persistent settings and the generated Root CA are stored in your operating system's user data directory (resolved by [`ca::data_dir()`](../app/src/ca.rs)):

| Operating System | Data Directory Path |
| :--- | :--- |
| **macOS** | `~/Library/Application Support/zniff` |
| **Windows** | `%APPDATA%\zniff` (e.g., `C:\Users\<username>\AppData\Roaming\zniff`) |
| **Linux** | `$XDG_DATA_HOME/zniff` (falls back to `~/.local/share/zniff`) |

### Files in the Data Directory

| File Name | Purpose |
| :--- | :--- |
| `config.toml` | **Production configuration file** (default port `8877`): stores listener settings, window/table layout, filter expression, TLS bypass hosts, rules, upstream proxy, and saved setups (`saved_profiles`). |
| `proxy-snapshot.toml` | **Production system-proxy recovery snapshot**: backs up original OS proxy settings before applying the system proxy. If `zniff` exits unexpectedly, the next launch automatically restores the original OS settings from this file. |
| `config.dev.toml` | **Development configuration file** (`make dev`, debug builds, or `ZNIFF_DEV=1`, default port `8878`): isolated from `config.toml` so development runs never overwrite your production setup. |
| `proxy-snapshot.dev.toml` | **Development system-proxy recovery snapshot**. |
| `ca.crt` / `ca.key` | Local `zniff CA` root certificate and private key (shared between production and development modes). |

> **Temporary Large-Body Spill Store**: During capture, completed unpinned request/response bodies larger than `16 KB` are appended to a rotating ring buffer in the system temp directory (e.g., `$TMPDIR/zniff-spill/`) and automatically cleaned up on exit or expiry.

---

## 2. Traffic Filter Syntax

The filter box at the top of the left pane and the **Traffic Filter** section in the top **Filters** popover stay synchronized in real time. Filters use **space-separated `AND` tokens**, with `|` or `,` inside a token for **`OR`**:

| Token / Prefix | Description | Example |
| :--- | :--- | :--- |
| `host:<domain>` or `domain:<domain>` | Match host substring or wildcard (`*.`), supports `\|` or `,` for `OR` | `host:api.example.com\|*.cdn.com` |
| `path:<prefix>` | Match request path prefix, supports `\|` or `,` for `OR` | `path:/api/v1\|/auth\|/graphql` |
| `ip:<addr>` or `client:<addr>` / `app:<name>` or `proc:<name>` | Match client source IP address or local process name, supports `\|` or `,` for `OR` | `ip:127.0.0.1\|192.168.1.25`, `app:curl\|node` |
| `GET` / `POST` / `PUT` / `DELETE` / `CONNECT` … | Exact HTTP method match | `POST host:api.example.com` |
| `200` / `404` / `1xx`–`5xx` | Exact status code or status class (`1xx`–`5xx`) | `5xx` or `404` |
| `fail` / `error` / `err` / `失败` | Match failed flows (`ERR` or status `>= 400`) | `fail` |
| `type:<kind>` or `ct:<kind>` | Match response `Content-Type` substring | `type:json`, `ct:html`, `type:image` |
| `body:<keyword>` | Search decompressed request and response bodies (including disk-spilled bodies) | `body:access_token` |
| `header:<keyword>` | Search request and response header names and values | `header:authorization`, `header:x-trace-id` |
| `note:<keyword>` or `comment:<keyword>` | Match flows by note/comment substring (or bare `note:` for any flow with a note) | `note:bug`, `note:` |
| `pin:` / `star:` | Show only starred flows | `pin:` |
| `slow:` | Show only slow flows (elapsed time `>= 1s`) | `slow:` |
| `mute:<term>` or `-<term>` | Exclude flows whose host or path matches `<term>`, supports `\|` for `OR` | `mute:ads.example.com\|/health` or `-telemetry` |
| Bare word | Case-insensitive substring match against host, path, or flow note | `login` |

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
- **Separators & wildcards**: Separate entries with spaces, commas `,`, semicolons `;`, or pipes `|`. Supports exact hosts, `*.example.com` (subdomains of `example.com`), and `*` (all hosts).

### Bypass Examples

```text
# Example 1: Skip TLS interception for Apple services and a pinned host
*.apple.com, *.icloud.com | pin.example.com

# Example 2: Allowlist mode — only decrypt api.example.com and *.dev.internal
only: api.example.com *.dev.internal
```

---

## 4. Traffic Rules Reference

Configured in **Rules** on the right side of the **Filters** popover (`rules` in `config.toml`).
The editor validates rule syntax in real time (highlighting any 1-based line numbers with syntax errors) and applies changes immediately to new requests without restarting the proxy.

### 4.1 Rule Syntax & `<pattern>` Matching

- **One rule per line**. Anything after `#` is treated as a comment.
- The second token of every rule is `<pattern>` (**no spaces allowed inside `<pattern>`**), written as `<host-pattern>[/<path-prefix>]`:
  1. **Host part**:
     - Exact host: `api.example.com`
     - Wildcard subdomain: `*.example.com` (matches `a.example.com`, `b.a.example.com`, not bare `example.com`)
     - All hosts: `*`
     - Multiple hosts (`OR`): joined with `|` or `,`, e.g., `api.example.com|*.staging.example.com`
  2. **Path prefix part** (optional; omitting it or writing `/` matches all paths on that host):
     - Matches on path segment boundaries (ignoring the `?query` part): `/api` matches `/api`, `/api/`, and `/api/v1/users`, but does **not** match `/apix`.
     - Multiple path prefixes (`OR`): joined with `|` or `,` after the first `/`, e.g., `example.com/api|/v2|/graphql`.
     - Any host with specific path prefixes: `*/api|/v2`.

### 4.2 Execution Model: Terminal vs. Modifier Rules

`zniff` supports 14 rule types divided into two categories:

1. **Terminal Rules**: `map-local`, `map-remote`, `break`, `break-res`, `block`
   - Determine how the request is fulfilled, paused, or terminated.
   - **First match wins** in top-to-bottom text order; subsequent terminal rules are ignored for that flow.
2. **Modifier Rules**: `replace`, `json`, `delay`, `throttle`, `header`, `query`, `cors`, `no-cache`, `cert`
   - **Stackable and non-terminal**. Every matching modifier rule is applied in order, even if it appears after the winning terminal rule.

---

### 4.3 All 14 Rule Types & Examples

#### ① `map-local` — Serve a Local File or Status-Only Mock Response

Answers the client directly from a local file (or with an empty body when only `status=<code>` is given) **without dialing the origin server**. The response `Content-Type` is inferred automatically from the file extension (`.json`, `.html`, `.js`, `.css`, `.xml`, `.png`, `.jpg`, `.svg`, `.webp`, `.gif`, `.txt`, etc.).

**Syntax:**
```text
map-local <pattern> [<local-file-path>] [status=<code-100..599>] [delay=<duration>]
```
> Note: Wrap `<local-file-path>` in double quotes (`"..."`) if the path contains spaces. `status=` defaults to `200` when omitted. Omitting `<local-file-path>` requires `status=<code>` and returns an empty response body with that status code. You can also right-click any captured flow and choose **Map local** to save its response body and append a `map-local` rule automatically.

**Examples:**
```text
# Serve a local JSON file with default status 200
map-local api.example.com/v1/profile ./mocks/profile.json

# Serve a local file whose path contains spaces
map-local api.example.com/v1/profile "/Users/alice/My Mocks/profile.json" status=200

# Status-only mock response without creating an empty file
map-local api.example.com/v1/maintenance status=503

# Simulate rate limiting or server errors (HTTP 429 after a 1.5s delay)
map-local api.example.com/v1/orders /tmp/rate_limit.json status=429 delay=1500ms

# Mock the same endpoint across multiple environments
map-local dev.example.com|staging.example.com/config /tmp/app-config.json
```

---

#### ② `map-remote` — Forward to Another Origin / URL

Redirects matching requests to a different destination URL (supports switching between `http://` and `https://`, updates the `Host` header, and preserves the remaining sub-path and query string).

**Syntax:**
```text
map-remote <pattern> <http://-or-https://-target-url>
```

**Path Joining Behavior:**
- If `<pattern>` is `example.com/api` and the target is `https://staging.internal:8443/v2`, a request to `https://example.com/api/users?id=1` is forwarded to `https://staging.internal:8443/v2/users?id=1`.

**Examples:**
```text
# Route production /api calls to a local backend running on port 3000
map-remote api.example.com/api http://127.0.0.1:3000/api

# Redirect an entire domain to a staging HTTPS origin
map-remote test.example.com https://staging-api.example.com
```

---

#### ③ `break` — Request Breakpoint (Pause Before Forwarding)

Pauses matching requests before they are sent to the origin server. A yellow breakpoint bar appears at the top of the window:
- Click **Edit** (or open the compose editor) to modify the **HTTP method, URL, request headers, and request body**, then click **Continue** (`放行`), **Continue all** (`全部放行`), or **Drop** (`丢弃`) to abort with `204 No Content`.
- Request and response breakpoints share a bounded **FIFO queue of up to 8 flows**: concurrent hits wait in order (`(1 / N)` shown in the breakpoint bar) without evicting the flow you are currently editing. Paused flows auto-release after 60 seconds of inactivity at the front of the queue (typing in the breakpoint editor automatically resets the 60s timer).

**Syntax:**
```text
break <pattern>
```

**Examples:**
```text
# Pause checkout requests to inspect or tamper with the payload before sending
break api.example.com/v1/checkout

# Pause login requests on any domain
break */auth/login
```

---

#### ④ `break-res` (or `break-response`) — Response Breakpoint (Pause Before Returning)

Forwards the request to the origin normally, buffers the response (up to `1 MB`), and pauses before returning it to the client:
- Edit the **HTTP status code** (e.g., change `200` to `500` or `403`), **response headers**, and **response body**.
- Click **Continue** (`放行`) in the breakpoint bar or editor header (or press `Enter` in the status code field) to deliver the modified response to the client.

**Syntax:**
```text
break-res <pattern>
```

**Examples:**
```text
# Pause the permissions response to test how the UI handles 403/500 or modified JSON
break-res api.example.com/v1/permissions
```

---

#### ⑤ `replace` — Text Substitution in Request Target/Body or Response Body

Substitutes UTF-8 text on the fly (stackable; multiple matching `replace` rules run in order):
- **`req` (request side)**: Rewrites both the **request target (`/path?query`)** and the **request body** (for UTF-8 bodies with `Content-Length` up to `1 MB`). Even requests without a body (such as `GET`) have their `/path?query` rewritten (as long as the resulting target still starts with `/`).
- **`res` (response side)**: Rewrites UTF-8 response bodies with `Content-Length` up to `1 MB` and recalculates `Content-Length`.
- `<from>` is a single non-whitespace token; leaving `<to>` empty after `->` **deletes** `<from>`.

**Syntax:**
```text
replace <pattern> req|res <from> -> <to>
```

**Examples:**
```text
# Replace a JSON field value in the response body
replace api.example.com/v1 res "env":"prod" -> "env":"debug"

# Rewrite query parameters or request body text from draft=1 to draft=0
replace api.example.com/articles req draft=1 -> draft=0

# Strip a query parameter from the request target (empty right-hand side)
replace api.example.com/api req ?debug=true ->
```

---

#### ⑥ `delay` — Latency Injection

Delays matching requests before forwarding. Multiple matching `delay` rules accumulate, capped at **30 seconds (`30s` / `30000ms`)** per flow.

**Syntax:**
```text
delay <pattern> <duration-in-ms-or-s>
```

**Examples:**
```text
# Add a 2-second delay to feed requests to test loading states
delay api.example.com/v1/feed 2000ms

# Add a 5-second delay to uploads
delay api.example.com/v1/upload 5s
```

---

#### ⑦ `header` — Set or Remove Request / Response Headers

Adds/overwrites (`set` or `add`, case-insensitive replacement) or removes (`del`, `delete`, or `rm`) headers on the request (`req`) or response (`res`).

**Syntax:**
```text
header <pattern> req|res set <Header-Name> <Header-Value>
header <pattern> req|res del <Header-Name>
```

**Examples:**
```text
# Inject a custom debug header into upstream requests
header api.example.com req set X-Debug-Env staging-gray

# Strip Cookie header from public API requests
header api.example.com/public req del Cookie

# Remove CSP or override caching headers on responses
header *.example.com res del Content-Security-Policy
header *.example.com res set Cache-Control no-store
```

---

#### ⑧ `query` — Set or Remove URL Query Parameters

Adds/overwrites (`set` or `add`) or removes (`del`, `delete`, or `rm`) query string parameters on the request URL before forwarding, handling `?` / `&` separators and percent-encoding automatically.

**Syntax:**
```text
query <pattern> set <Param-Name> <Param-Value>
query <pattern> del <Param-Name>
```

**Examples:**
```text
# Force env=staging on all /v1 API requests
query api.example.com/v1 set env staging

# Remove tracking parameters before forwarding
query *.example.com del utm_source
```

---

#### ⑨ `cors` — Permissive CORS Injection & Preflight Short-Circuit

Ensures permissive CORS headers are present on matching responses (preserving any existing CORS headers already sent by the origin):
- `Access-Control-Allow-Origin: *`
- `Access-Control-Allow-Methods: GET, POST, PUT, PATCH, DELETE, OPTIONS, HEAD`
- `Access-Control-Allow-Headers: *`
- `Access-Control-Max-Age: 86400`

Additionally, if an **`OPTIONS` preflight request** matches `cors` and no terminal rule (`map-local`, `map-remote`, `break`, `break-res`) matches, `zniff` **immediately responds with `204 No Content`** and the CORS headers without contacting the origin.

**Syntax:**
```text
cors <pattern>
```

**Examples:**
```text
# Enable CORS for a specific API domain
cors api.example.com

# Enable CORS for all hosts
cors *
```

---

#### ⑩ `block` — Block Traffic with Status Code or Connection Drop/Reset

Interceps matching requests at the proxy level without contacting the origin:
- Return a specific HTTP status code (e.g., `status=403`, `status=500`), with an optional response body.
- Or drop/reset the connection immediately using `drop` or `reset` (recorded with status `规则阻断 (连接已断开)`).

**Syntax:**
```text
block <pattern> [status=<code-100..599>] [drop|reset]
```

**Examples:**
```text
# Return HTTP 403 Forbidden to tracking domains
block telemetry.example.com status=403

# Immediately sever connections to ad servers
block ads.example.com drop
```

---

#### ⑪ `json` — Modify or Delete JSON Fields by JSON Pointer or Dot Path

Modifies or removes fields within JSON request or response bodies (up to `1 MB`) on the fly, preserving key ordering:
- Use standard JSON Pointer (`/path/to/field`) or dot notation (`path.to.field`).
- Use `set` with a JSON literal (`true`, `false`, `null`, `123`, `"value"`, `{"a":1}`) or `del` to remove.

**Syntax:**
```text
json <pattern> req|res set <pointer-or-dot-path> <json-value>
json <pattern> req|res del <pointer-or-dot-path>
```

**Examples:**
```text
# Grant VIP status in user profile response
json api.example.com/v1/profile res set /data/is_vip true

# Delete tracking field from request payload
json api.example.com/events req del /metadata/device_fingerprint
```

---

#### ⑫ `throttle` — Bandwidth Throttling & Network Simulation

Simulates real-world poor network conditions using 4 KB chunked streaming delays:
- Presets: `2g` (50 kbps, 500ms RTT), `3g` (750 kbps, 100ms RTT), `4g` (4 Mbps, 20ms RTT).
- Or directional custom rates: `down=<rate>` and `up=<rate>` (e.g., `down=256k up=64k`).

**Syntax:**
```text
throttle <pattern> <preset|down=<rate> up=<rate>>
```

**Examples:**
```text
# Simulate 3G mobile network on mobile API endpoints
throttle api.example.com/mobile 3g

# Limit media downloads to 500 KB/s
throttle cdn.example.com/video down=500k
```

---

#### ⑬ `no-cache` — Strip Conditional Caching Headers & Force Fresh Responses

Forces clients and servers to bypass HTTP caching:
- Strips `If-Modified-Since` and `If-None-Match` from requests before forwarding to prevent `304 Not Modified`.
- Injects `Cache-Control: no-cache, no-store, must-revalidate` and `Pragma: no-cache` into responses.

**Syntax:**
```text
no-cache <pattern>
```

**Examples:**
```text
# Disable caching for static assets during frontend development
no-cache static.example.com/assets
```

---

#### ⑭ `cert` / `client-cert` — Mount Custom Domain Certificate or Upstream mTLS

Provides fine-grained TLS certificate control:
- Mount custom server certificates for specific domains instead of generating leaf certificates with the local root CA.
- Or inject upstream client certificates (`client-cert` or `mtls`) for origins requiring two-way mutual TLS (mTLS) authentication.

**Syntax:**
```text
cert <pattern> <cert-path> <key-path>
```

**Examples:**
```text
# Provide custom company wildcard cert for internal services
cert *.internal.corp /etc/ssl/internal.crt /etc/ssl/internal.key

# Authenticate against upstream banking API using client mTLS certificate
cert secure.bank.com/api ./certs/client.pem ./certs/client.key
```

---

### 4.4 GraphQL Operation Matching (`op=<GraphQL-Operation>`)

11 of the 14 rule types (`map-local`, `map-remote`, `break`, `break-res`, `block`, `replace`, `json`, `delay`, `header`, `query`, `no-cache`) support narrowing their scope by GraphQL operation name:
- Append `op=<name>` to the rule.
- Matches against explicit `operationName`, inferred named queries/mutations/subscriptions, or `anonymous`.
- Supports `*` wildcards and `|` OR unions.

**Examples:**
```text
# Mock only the 'GetUserInfo' GraphQL query
map-local api.example.com/graphql ./mocks/user.json op=GetUserInfo

# Inject a 3s delay specifically on mutation operations starting with 'Submit*'
delay api.example.com/graphql 3000ms op=Submit*

# Set breakpoint on Checkout or Payment operations
break api.example.com/graphql op=Checkout|Payment
```

---

## 5. Global Switches & Upstream Proxy

At the top of the right column in the **Filters** popover are three switches:

1. **Rules (`启用规则`)**: Temporarily disables all traffic rules without erasing your rule text or modifying `config.toml`. The toolbar **Filters** button turns amber while disabled.
2. **Breaks (`启用断点`)**: Temporarily skips `break` and `break-res` pauses while keeping `map-local`, `map-remote`, `replace`, `delay`, `header`, and `cors` active.
3. **Tunnel errors (`隧道失败`)**: Off by default. When enabled, origin TLS probe failures that fall back to a TCP tunnel are recorded as rows in the flow list for troubleshooting.

### Upstream Proxy (`upstream_proxy`)

Enter an upstream HTTP/CONNECT proxy address in the **Upstream Proxy** field at the bottom of the right column (`upstream_proxy` in `config.toml`), or leave it blank to connect directly to origins:

```text
127.0.0.1:7890
# or with scheme
http://proxy.internal.corp:8080
```

---

## 6. Saved Setups (`SavedProfile`) & TOML Import/Export

The bottom bar of the **Filters** popover lets you save the entire working state (**Filter expression + TLS Bypass + Rules + Upstream Proxy + 3 toggle switches**) as a named setup (up to 16 setups stored under `[[saved_profiles]]` in `config.toml`):

- **Switch Setup**: Pick any saved setup from the dropdown on the left to populate all fields and apply them immediately.
- **Save**: Enter a name and click **Save** (any filled draft rule in the Quick Add row is automatically appended before saving). Saving with an existing name (case-insensitive) updates that setup.
- **New**: Click **New** to clear the working filter, bypass list, rules, and upstream proxy and reset the switches to defaults without deleting your saved setups.
- **Export & Import `.toml`**:
  - **Export** serializes the setups as `.toml` text and copies it directly to the clipboard.
  - **Import** opens a multiline input textarea where you can paste `.toml` setup content and click **Confirm Import** (you can also **drag and drop a `.toml` file directly onto the `zniff` window**).

### Example Exported Setup (`.toml`)

```toml
[[profiles]]
name = "Local API Dev"
filter = "host:api.example.com|127.0.0.1 mute:/health"
bypass_hosts = "*.apple.com, *.icloud.com"
rules = """
# Forward v1 to local server and mock feature flags
cors api.example.com
header api.example.com req set X-Env local-dev
map-remote api.example.com/v1 http://127.0.0.1:3000/v1
map-local api.example.com/v2/feature-flags ./mocks/flags.json
"""
upstream_proxy = ""
```
