# zniff

English | [简体中文](./README-zh.md)

**zniff** is a lightweight HTTP(S) debugging proxy written in Rust. The desktop app runs the proxy and the native UI (`nsh`) in one process. The same UI also compiles to WebAssembly and is served by `zniff --web`; that server process does the capture. Sessions stay in memory, with no external database.

---

## Desktop and web

![The code zniff shares between the desktop app and the web version](./docs/zniff-architecture.svg)

The UI (`App`) and the capture layer (`Hub` plus TLS, HTTP/2, and rules) are written once and compiled into each program.

| | Desktop | Web |
| :--- | :--- | :--- |
| UI | Native window in the same process | The same `App`, as WebAssembly in the browser |
| Capture | The UI calls `Hub` in-process | The page calls `zniff --web` over `/api` |
| Who is captured | Clients that use this process as their HTTP proxy | The same, on the server |
| System proxy and root CA | This machine | Stay on the desktop app |

`make web` serves the shared page at `http://127.0.0.1:8080`. The page is the operator UI. Capture runs in the `zniff --web` process, and only for clients that set that process as their HTTP proxy. The capture port (dev config `listen_port`, otherwise `8877`) stays closed until **Start**. `zniff --web` does not change the system proxy and does not follow the desktop `capturing` flag.

`make web-ui` rebuilds that page with nsh 0.5 into `app/.nsh/web`. After a UI change, run it again and reload the browser. The installed `nsh` may be 0.2; packaging needs 0.5 and must not replace `~/.nsh`. Library tests still use the small console and do not load `app/.nsh/web`.

A relay is optional and off by default. With a relay URL and token, a live share is pushed to that host about once a second. `zniff --relay --token <secret>` only stores and serves `/s/{id}`. Another zniff can accept the same uploads on its capture port when **Receive relay** is on. Those rows can be viewed, filtered, copied, and starred. Replay stays on the machine that captured them.

The web server can host the relay as well, on its own port:

| Command | Capture | Relay |
| --- | --- | --- |
| `zniff --web` | after **Start** | — |
| `zniff --web --relay --token <secret>` | after **Start** | port `8879` (`--relay-port`, `--relay-bind`, default `0.0.0.0`) |
| `zniff --web --relay --token <secret> --no-capture` | never; starting returns `403` | port `8879` |

Each port checks its own credential, and none of them works on another port:

| Port | Check | Source |
| --- | --- | --- |
| Console (`8080`) | HTTP Basic on every request, when set | `--web-auth <user:pass>` › `ZNIFF_WEB_AUTH` › `web_auth` in `config.toml` |
| Relay upload / delete | `Authorization: Bearer <token>`, required | `--token` › `relay_token` |
| Relay share pages | HTTP Basic, when set | `--view-auth <user:pass>` › `ZNIFF_RELAY_VIEW_AUTH` › `relay_view_auth` |
| Capture (`8877`) | `Proxy-Authorization`, when set | `--proxy-auth` › `ZNIFF_PROXY_AUTH` › `proxy_auth` |

A wrong or missing credential gets `401` after about 300 ms. `--view-auth` works the same with the standalone `zniff --relay`. The desktop app still sends only the relay URL and token; give viewers the view password yourself. Basic auth is plain text over HTTP, so put an HTTPS reverse proxy in front of anything public.

---

## Features

### 1. Protocol-Following HTTP(S) & WebSocket Proxy
- **Protocol Following (HTTP/1.1 & HTTP/2)**: Reads the client's TLS `ClientHello` ALPN offers, negotiates with the origin server directly via `rustls` and `hyper`, and uses the origin-selected protocol on both sides without downgrading HTTP/2 to HTTP/1.1 or altering header casing.
- **Side-Copy Body Recording & Disk Spill**: Forwarding never waits for UI rendering or decompression. Request and response bodies are decompressed (`gzip`, `deflate`, `br`, `zstd`) on a side copy (up to `1 MB` per body). Completed unpinned flows with bodies over `16 KB` automatically spill to an anonymous/auto-cleaned ring buffer on disk (`zniff-spill/`, up to `512 MB` across 8 segments) to keep memory usage low (`256 MB` session cap).
- **WebSocket & SSE Inspection**: Parses WebSocket frames (`text`, `binary`, `ping`, `pong`, `close`) from a side stream without blocking forwarding, and extracts Server-Sent Events (`text/event-stream`) into structured message lists.
- **TLS Bypass & Fallback**: Supports wildcard domain bypass/allowlist rules (`*.example.com`, `only:`) evaluated against SNI or `CONNECT` host. Pre-`ServerHello` handshake failures automatically fall back to a transparent TCP tunnel.
- **Upstream Proxy**: Optional upstream HTTP/CONNECT proxy chaining (`upstream_proxy`).

### 2. Fast Filtering & Customizable Flow Table
- **Customizable Columns**: Toggle, resize, and reorder columns (by dragging table headers directly or using the `↑`/`↓` buttons in the column picker) in matching table/menu order: `#` (`id`), `Method`, `Path` (or full `URL`), `Status`, `Gap`, `Elapsed`, `Size`, `Protocol`, `Host`, and `Client IP`. `#`, `Method`, and `Path` stay pinned in the left fixed zone during horizontal scrolling.
- **Rich Filter Syntax**: Space-separated `AND` terms with `|` or `,` for `OR` within a term:
  - `host:api.example.com|*.cdn.com` (or `domain:`)
  - `path:/v1|/auth`
  - `ip:127.0.0.1|192.168.1.10` (or `client:`), `app:curl|node` (or `proc:`, matches local client process name on loopback connections)
  - Exact HTTP method (`GET`, `POST`, `PUT`, `DELETE`, `CONNECT`, …)
  - Status code (`200`, `404`), status class (`1xx`–`5xx`), or failed flows (`失败` / `fail` / `error` / `err`)
  - `type:json` (or `ct:html`, `ct:image`, …)
  - `body:keyword` (searches request and response bodies)
  - `header:authorization` (searches header names and values)
  - `note:keyword` (or `comment:`, matches flow notes; bare `note:` matches any annotated flow)
  - `pin:` (starred flows), `slow:` (elapsed ≥ 1s)
  - `mute:ads.example.com` or `-keyword` (exclude matching host or path)

### 3. Traffic Rules, Breakpoints & Saved Setups
> Full guide with syntax details and practical examples: **[Rules & Configuration Guide](./docs/rules.md)** ([中文指南](./docs/rules-zh.md))

- **Terminal Rules** (first match wins):
  - `map-local <pattern> [<file>] [status=200]` — Serve a local file (or status-only empty response) without dialing the origin. Right-click any captured flow and choose **Map local** to save its response and create a rule in one step.
  - `map-remote <pattern> <https://target-host/path>` — Rewrite the destination origin/URL.
  - `break <pattern>` — Pause the request before forwarding to inspect or edit method, URL, headers, and body.
  - `break-res <pattern>` — Pause the response before returning to the client to edit status code, headers, and body.
- **Modifier Rules** (stackable):
  - `replace <pattern> req|res <from> -> <to>` — Rewrite request target/body or UTF-8 response body (supports `"..."` quoted strings with spaces).
  - `delay <pattern> <ms|s>` — Inject latency up to 30 seconds.
  - `header <pattern> req|res set|del <Name> [<Value>]` — Set or remove request/response headers.
  - `query <pattern> set|del <Name> [<Value>]` — Set or remove URL query parameters automatically.
  - `cors <pattern>` — Inject permissive CORS headers and auto-answer `OPTIONS` preflight with `204`.
- **Saved Setups (`SavedProfile`)**: Save, switch, export to clipboard (`.toml`), and import complete setups (via the multiline TOML textarea or by dropping `.toml` files) combining filter expressions, bypass hosts, rules, upstream proxy, and toggle states.

### 4. Deep Inspection, Diff & Replay
- **Smart Body Viewer**:
  - Syntax-highlighted read-only editor for JSON, HTML, XML, and multi-line text, with an inline **Search** box and hit-count filter in both request and response body headers.
  - Zero-config **GraphQL** query/variables formatting, schema-less **Protobuf / gRPC** wire decoding, and **MessagePack** to JSON decoding in **Pretty** mode.
  - Rendered **Preview** (`text_view`) for HTML and Markdown bodies alongside **Pretty** and **Raw** modes.
  - Inline image preview (`png`, `jpeg`, `webp`, `gif`) with pixel dimensions and byte size.
  - Responsive 16-byte / 32-byte hex viewer for binary payloads.
  - Automatic inline annotations for **JWT** payloads, Unix **timestamps**, and printable **Base64** strings in headers, query parameters, and form fields (`application/x-www-form-urlencoded` and `multipart/form-data`).
- **Flow Notes & Clear Unstarred**: Add searchable notes (`# note`) to any flow (persisted in HAR 1.2 `comment`), and clear unstarred noise while keeping starred flows intact.
- **Line-by-Line Diff**: Mark any flow as a **Baseline** (red indicator) and select another flow (green indicator) to compare URL, request headers/body, and response headers/body side by side.
- **Filtered Summary & Sequence Diagram Generator**: When no flow is selected (or after pressing `Esc` / clicking **Summary** in the detail header), the right pane displays a **Filtered summary** card with total requests, transfer size (`↑req · ↓res`), average latency, status code breakdown badges (`2xx`, `3xx`, `4xx`, `5xx`, `ERR`), a clickable **Slowest** flow shortcut, and a **Sequence diagram** builder (also in the **More** menu) that loads all currently filtered flows checked by default, lets you check/uncheck individual flows or toggle **Responses** and **Query string**, and generates a Mermaid `sequenceDiagram` with one-click **Copy Mermaid** and **Copy Markdown** actions.
- **Replay, Batch Repeat & Standalone Compose**: One-click **Replay** (`Cmd/Ctrl+R`), **Repeat…** (batch replay `1..=100` times with configurable interval in ms and `5×` / `10×` / `20×` presets), **Edit** an existing flow, or click **New request** (in the empty detail pane or **More** menu) to compose a request from scratch or paste a `curl ...` command into the URL field to populate all fields automatically.
- **Code & Snippet Export**: Copy as URL, `cURL`, JavaScript `fetch`, Python `requests`, shallow TypeScript `interface` definitions, or one-click terminal proxy & CA environment variables (`HTTP_PROXY`, `ALL_PROXY`, `SSL_CERT_FILE`, `NODE_EXTRA_CA_CERTS`, `REQUESTS_CA_BUNDLE`, `CURL_CA_BUNDLE`, `GIT_SSL_CAINFO`, `CARGO_HTTP_CAINFO`, `DENO_CERT`, `JAVA_TOOL_OPTIONS`).

### 5. Mobile Capture, HAR & Live Sharing
- **System Proxy & Root CA Diagnosis**: Generates a local `zniff CA` root certificate (`ca.crt` / `ca.key`) with one-click installation/removal for macOS Keychain, Windows Root Store, and Linux NSS (`~/.pki/nssdb`), and diagnoses certificate trust status (`Trusted`, `InstalledUntrusted` via `security verify-cert`, or `NotInstalled`). System proxy changes are tracked in a separate `proxy-snapshot.toml` and automatically restored if the process exits unexpectedly.
- **LAN & Mobile QR Code**: Enable **LAN** (`0.0.0.0`) in the footer and click **QR** to scan and open `http://<lan-ip>:<port>/zniff` on iOS/Android for guided root certificate installation.
- **HAR 1.2 & Standalone HTML**: Import `.har` files (via menu or drag-and-drop) and export filtered flows to `.har` or self-contained `.html` snapshots.
- **Live Web Share**: Share a read-only live view (`15 min`, `1 hour`, or `8 hours`) at `http://<addr>:<port>/s/<uuid>` with incremental delta updates and on-demand body loading.

---

## Keyboard Shortcuts

| Shortcut | Action |
| :--- | :--- |
| `↑` / `↓` | Select previous / next flow in the filtered list |
| `Cmd/Ctrl + F` | Focus the filter input box |
| `Cmd/Ctrl + K` | Clear the entire capture session |
| `Cmd/Ctrl + R` | Replay the currently selected flow |

---

## Configuration & Data Directory

User settings are stored in `config.toml` and system proxy recovery state in `proxy-snapshot.toml`:

- **macOS**: `~/Library/Application Support/zniff`
- **Windows**: `%APPDATA%\zniff`
- **Linux**: `$XDG_DATA_HOME/zniff` or `~/.local/share/zniff`

> **Development Mode Isolation**: Running in debug mode or with `ZNIFF_DEV=1` (`make dev`) uses `config.dev.toml` (default port `8878`) and `proxy-snapshot.dev.toml` so development never overwrites your production `config.toml` (default port `8877`), while sharing `ca.crt` and `ca.key`.

---

## Development & Build

Run all commands from the repository root (`app/` is the single crate and workspace):

```bash
# Desktop window. bacon stays attached. Dev config, default port 8878
make dev

# Shared UI at http://127.0.0.1:8080. Capture stays closed until Start
make web

# Rebuild app/.nsh/web after a UI change (nsh 0.5)
make web-ui

# Extra flags, for example another web port
make web ARGS="--web-port 8090"

# Build debug binary
make build

# Format, lint (warnings denied), and run unit/integration tests
make fmt
make lint
make test

# Build optimized release binary
make release

# Package macOS application bundle (zniff.app)
make bundle
```

The daily image is [`vicanso/zniff-web:nightly`](https://hub.docker.com/r/vicanso/zniff-web) (linux/amd64 and linux/arm64). A stable tag also publishes `vicanso/zniff-web:<version>` and `latest`. The page in the image is the small console. Mount a `make web-ui` directory to `/web` for the shared UI, and mount `/data` (`ZNIFF_DATA_DIR=/data` in the image) to reuse the generated `ca.crt`/`ca.key` and `config.toml` across container restarts (optional `--proxy-auth` / `ZNIFF_PROXY_AUTH` requires HTTP Basic auth on port `8877`):

```bash
docker pull vicanso/zniff-web:nightly
docker run --rm -p 8080:8080 -p 8877:8877 -v "$PWD/zniff-data:/data" vicanso/zniff-web:nightly

docker run --rm -p 8080:8080 -p 8877:8877 \
  -e ZNIFF_WEB_ROOT=/web \
  -e ZNIFF_PROXY_AUTH=user:pass \
  -v "$PWD/app/.nsh/web:/web:ro" \
  -v "$PWD/zniff-data:/data" \
  vicanso/zniff-web:nightly
```

Build it locally with `docker build -t zniff .`. The entrypoint is `zniff --web --web-bind 0.0.0.0 --bind 0.0.0.0 --port 8877`. The capture port still opens only after **Start**. Keep `8080` and `8877` off the public internet: the page can show bodies and cookies. If the console must be reachable, set `ZNIFF_WEB_AUTH=user:pass`; without it the server prints a warning when the console is bound to a non-loopback address. The image also exposes `8879` for `--relay`, extra `docker run` arguments are appended to the entrypoint, for example `docker run -p 8080:8080 -p 8879:8879 -e ZNIFF_WEB_AUTH=op:pw <image> --relay --token <secret> --no-capture`. Publishing needs `DOCKER_HUB_USERNAME` and `DOCKER_HUB_ACCESS_TOKEN`; without them CI still builds the image and does not push it.

See [docs/known-issues.md](./docs/known-issues.md) for documented platform boundaries and design trade-offs.

---

## License

Licensed under the [Apache License, Version 2.0](./LICENSE).
