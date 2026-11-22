# zniff User Guide

English | [简体中文](./guide-zh.md)

A tour of what zniff can do, how to point clients at it, and how to move over from Charles or Whistle. For the full rule syntax see the [Rules & Configuration Guide](./rules.md); for headless modes see [CLI, Web Console & MCP](./cli-web-mcp.md).

## Contents

- [Features](#features)
- [Install](#install)
- [Client & Root CA Setup](#client--root-ca-setup)
- [Migrate from Charles / Whistle](#migrate-from-charles--whistle)
- [Rewrite Rules at a Glance](#rewrite-rules-at-a-glance)
- [Keyboard Shortcuts](#keyboard-shortcuts)
- [Data Directory & Memory](#data-directory--memory)
- [Build & Test](#build--test)

## Features

- **Native & Zero-Config Capture**: Pure-Rust single binary (proxy + `nsh 0.6.1` GUI, bounded memory with disk spill). Launch an **isolated Chromium browser** (SPKI trust, no OS keychain edits), **proxied terminal**, **Android `adb` reverse proxy**, **iOS Simulator**, or phone QR from the **Connect** hub—or click **Load Sample Traffic** on the empty onboarding card to explore immediately without configuring a proxy.
- **Self-Healing Diagnostics & Smooth Daily UX**: Failed (`ERR` / `407`) flows show one-click repair buttons (**Bypass TLS**, **Install CA**, **Open in Browser**, **Check Upstream**, **Add resolve**, **Open Security**). Empty filtered views diagnose which filter token blocked traffic with quick reset/pause actions; clearing a session offers a **5-second Undo toast**; press `?` or `Cmd/Ctrl+/` for the shortcuts cheatsheet and `Cmd/Ctrl+Shift+P` for the Command Palette.
- **True Protocol Following & SOCKS5 Upstream**: Reads `ClientHello` ALPN offers and negotiates directly with origins for HTTP/2 multiplexing, WebSocket (RFC 6455), and SSE without HTTP/1.1 downgrades; upstream chaining supports HTTP/CONNECT, `socks5://`, and `socks5h://`.
- **Deep Inspection, AI SSE & Schema Decoding**: Pretty JSON + searchable JSON tree, GraphQL (`operationName` + query/variables), **LLM / AI SSE Stream Inspector** (OpenAI, Anthropic, Gemini thinking/output/tool calls/TTFT/tok/s), HTML/Markdown preview, XML, MessagePack, zero-dependency `.proto` schema decoding, inline JWT/timestamp/Base64 annotations, baseline Diff, and timing waterfalls.
- **17 Rule Types & Multi-Group Stacking**: Independently toggleable rule groups (`[[rule_groups]]`), terminal (`map-local`, `map-remote`, `break`, `break-res`, `block`), additive (`replace`, `json`, `delay`, `throttle`, `header`, `query`, `cors`, `no-cache`), host-level (`cert`, `client-cert`, `resolve`), and extension rules (`hook` mutate/preview, `proto`), with `{{uuid}}` / `{{hmac_sha256(...)}}` templates, GraphQL `op=` targeting, and an 8-slot FIFO breakpoint queue.
- **Diagrams, OpenAPI, Mock & Collaboration**: Interactive Waterfall, Swimlane Sequence, Dependency (`flowchart TD`), and Mermaid views; copy as cURL, `fetch`, Python, Rust, Go, TypeScript interfaces, or Rust `serde` structs; export OpenAPI 3.0.3, batch Local Mock directories, redacted HAR/HTML, or live `/s/{uuid}` & `--relay` shares.
- **AI-Native MCP & Web Console**: Built-in JSON-RPC 2.0 MCP server (`POST /mcp` and `zniff --mcp`, 10 tools) plus a WebAssembly console (`zniff --web`) with Basic auth, capture-port `allow_hosts` control (strict deny-all default in Docker), and verified in-place updates via `SHA256SUMS`. See [CLI, Web Console & MCP](./cli-web-mcp.md).

Filter syntax (`host:`, `path:`, `status:`, `body:`, `op:`, `-term`, …) is documented in [Rules & Configuration → Traffic Filter Syntax](./rules.md#2-traffic-filter-syntax).

## Install

Download pre-built packages from [GitHub Releases](https://github.com/vicanso/zniff/releases): signed `.dmg` for macOS (Apple Silicon / Intel), `.tar.gz` for Linux (x86_64 / aarch64) and Windows (x86_64 / aarch64, contains `zniff.exe`). On Linux, run the bundled `install-linux.sh` from the unpacked tarball for a user-level install (launcher entry and autostart; `--no-autostart`, `--uninstall`, `--tag vX.Y.Z`). Package manager manifest templates for **Homebrew Cask**, **Scoop**, and **Winget** live in [`packaging/`](../packaging/README.md). Or build from source:

```bash
cargo install --git https://github.com/vicanso/zniff
# Or clone and package macOS .app:
# git clone https://github.com/vicanso/zniff.git && cd zniff && make bundle
```

For the Docker web console, see [CLI, Web Console & MCP → Docker](./cli-web-mcp.md#docker).

## Client & Root CA Setup

1. Launch `zniff` (default capture address: `127.0.0.1:8877`).
2. **Zero-Config Capture (Recommended)**: Click **Connect** in the footer, use the empty-state onboarding card, or open **More → Launch** to start `[Browser]` (isolated Chromium with SPKI cert trust), `[Term]` (pre-configured proxy & CA env vars), `[ADB]` (USB reverse proxy for Android devices), or copy the dynamic `/proxy.pac` URL.
3. **Manual Terminal Setup**: Click **More → Copy Proxy Env** to copy proxy variables:
   ```bash
   export HTTP_PROXY="http://127.0.0.1:8877"
   export HTTPS_PROXY="http://127.0.0.1:8877"
   export ALL_PROXY="http://127.0.0.1:8877"
   export SSL_CERT_FILE="$HOME/Library/Application Support/zniff/ca.crt"
   ```
4. **Trust Root CA (for system-wide or mobile capture)**:
   - **macOS**: **More → Certificate → Install** to trust `zniff CA` in Keychain.
   - **External Root CA Reuse (Optional)**: Auto-detects trusted local root CAs from **Whistle**, **mitmproxy**, **mkcert**, or **Proxyman** via **More → Certificate** or the footer **Connect** popover (`external_ca`).
   - **iOS Simulator**: **More → Install CA to iOS Simulators** to inject the cert and proxy into all booted simulators via `xcrun simctl`.
   - **Mobile Devices**: Click **QR** in the footer or visit `http://<lan-ip>:8877/zniff`.

## Migrate from Charles / Whistle

Already have rules, sessions, or a trusted CA from Charles or Whistle? You can switch without rebuilding your setup:

1. **Reuse your existing Root CA (zero cert re-install)**: Open **Connect** in the footer or **More → Certificate** — zniff automatically detects trusted local root CAs from **Whistle**, **mitmproxy**, **mkcert**, and **Proxyman** so you can reuse them with one click.
2. **Import rules & captures by drag-and-drop**: Drop (or paste in **Filters → Import**) any **Whistle `.txt`**, **Charles `.xml`** (Map Local, Map Remote, Rewrite, Breakpoints), **`.chlsj`** session, or **`.har`** file directly onto the window. Unmapped directives are preserved as `#` comments so nothing is lost.
3. **Explore immediately**: Before changing any proxy settings, click **Load Sample Traffic** on the empty onboarding card—or launch the **Isolated Browser** / **Proxied Terminal** from **Connect**.

| Charles / Whistle | zniff Equivalent |
| :--- | :--- |
| Whistle `api.example.com file:///mocks/a.json` / Charles Map Local | `map-local api.example.com ./mocks/a.json` |
| Whistle `api.example.com http://127.0.0.1:3000` / Charles Map Remote | `map-remote api.example.com http://127.0.0.1:3000` |
| Whistle `api.example.com 127.0.0.1:8443` / DNS Spoofing | `resolve api.example.com 127.0.0.1:8443` |
| Whistle `resDelay://1500` / Charles Throttling | `delay api.example.com 1500ms` / `throttle api.example.com 256kb/s` |
| Charles Breakpoints | `break api.example.com/checkout` / `break-res api.example.com/checkout` |
| Charles SSL Proxying Exclude | Add host to **TLS Bypass** (or click **Bypass TLS** on any TLS `ERR` flow) |

## Rewrite Rules at a Glance

Rules are one per line. Multiple hosts or paths can be joined with `|` or `,`.

```text
# Terminal rules (first match wins)
map-local  api.example.com/v1/user ./mocks/user.json
map-remote api.example.com/api http://127.0.0.1:3000/api
break      api.example.com/v1/checkout
block      ads.example.com status=403

# Additive modifiers & dynamic template variables (accumulate in order)
json       api.example.com/v1 res set /data/is_vip true
delay      api.example.com/graphql 2s op=Update*
header     api.example.com/v1 req set X-Signature {{hmac_sha256("secret", req.body)}}

# Local HTTP webhook extension & .proto schema decoding
hook       api.example.com/v1/cipher http://127.0.0.1:9001/decrypt res mode=preview
proto      grpc.example.com ./protos
```

See the full syntax and examples in the **[Rules & Configuration Guide](./rules.md)**.

## Keyboard Shortcuts

| Shortcut | Action |
| :--- | :--- |
| `Cmd/Ctrl + Shift + P` | Toggle global Command Palette |
| `?` or `Cmd/Ctrl + /` | Open Keyboard Shortcuts cheatsheet |
| `Cmd/Ctrl + F` | Focus filter bar |
| `Cmd/Ctrl + K` | Clear session (5-second Undo toast) |
| `Cmd/Ctrl + R` | Replay selected flow |
| `Cmd/Ctrl + Enter` | Send composed request / Continue breakpoint |
| `↑` / `↓` | Navigate filtered flows |
| `Esc` | Close popover/modal or return to summary view |

## Data Directory & Memory

- **macOS**: `~/Library/Application Support/zniff`
- **Linux**: `$XDG_DATA_HOME/zniff` or `~/.local/share/zniff`
- **Windows**: `%APPDATA%\zniff`

Override with `--data-dir <dir>` or `ZNIFF_DATA_DIR` (the Docker image uses `/data`). Contains `config.toml`, `ca.crt` / `ca.key`, `proxy-snapshot.toml`, and `config.dev.toml` (dev mode, port `8878`). Sessions cap at 256 MB in memory, spilling unpinned bodies > 16 KB to a rotating 512 MB disk ring buffer (`zniff-spill/`).

## Build & Test

```bash
make dev      # Run desktop app in dev mode (port 8878)
make release  # Build release binary
make bundle   # Package macOS zniff.app
make web      # Run Web console (port 8080)
make test     # Run tests
make lint     # Run clippy
```
