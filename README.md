# zniff

English | [简体中文](./README-zh.md)

**zniff** is a lightweight, native desktop HTTP(S) debugging proxy and packet capture tool written in Rust. Both the proxy engine and the native GUI (`nsh`) run in a single process with zero external database dependencies.

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
- **Replay & Standalone Compose**: One-click **Replay** (`Cmd/Ctrl+R`), **Edit** an existing flow, or click **New request** (in the empty detail pane or **More** menu) to compose a request from scratch or paste a `curl ...` command into the URL field to populate all fields automatically.
- **Code & Snippet Export**: Copy as URL, `cURL`, JavaScript `fetch`, Python `requests`, or shallow TypeScript `interface` definitions.

### 5. Mobile Capture, HAR & Live Sharing
- **System Proxy & Root CA**: Generates a local `zniff CA` root certificate (`ca.crt` / `ca.key`) with one-click installation/removal for macOS Keychain, Windows Root Store, and Linux NSS (`~/.pki/nssdb`). System proxy changes are tracked in a separate `proxy-snapshot.toml` and automatically restored if the process exits unexpectedly.
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
# Run in development mode with hot-reload (port 8878)
make dev

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

See [docs/known-issues.md](./docs/known-issues.md) for documented platform boundaries and design trade-offs.

---

## License

Licensed under the [Apache License, Version 2.0](./LICENSE).
