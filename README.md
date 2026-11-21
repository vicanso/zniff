# zniff

<p align="center">
  <strong>Single-process HTTP(S) debugging proxy in pure Rust — native GUI, WebAssembly console, and built-in MCP server.</strong>
</p>

<p align="center">
  English | <a href="./README-zh.md">简体中文</a>
</p>

<p align="center">
  <a href="https://github.com/vicanso/zniff/releases"><img src="https://img.shields.io/badge/version-v0.4.0-blue.svg" alt="Version"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/rust-2024%20edition-orange.svg" alt="Rust 2024"></a>
  <a href="./LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-green.svg" alt="License"></a>
  <a href="#quick-start"><img src="https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows%20%7C%20Docker-lightgrey.svg" alt="Platform"></a>
  <a href="#cli-modes--mcp"><img src="https://img.shields.io/badge/MCP-JSON--RPC%202.0-blueviolet.svg" alt="MCP Ready"></a>
</p>

https://github.com/user-attachments/assets/15117c1d-50a0-4578-8201-a305fb0d12a3

---

## Features

- **Native & Zero-Config Capture**: Pure-Rust single binary (proxy + `nsh 0.6.1` GUI, bounded memory with disk spill). Launch an **isolated Chromium browser** (SPKI trust, no OS keychain edits), **proxied terminal**, **Android `adb` reverse proxy**, **iOS Simulator**, or phone QR from the **Connect** hub—or click **Load Sample Traffic** on the empty onboarding card to explore immediately without configuring a proxy.
- **Self-Healing Diagnostics & Smooth Daily UX**: Failed (`ERR` / `407`) flows show one-click repair buttons (**Bypass TLS**, **Install CA**, **Open in Browser**, **Check Upstream**, **Add resolve**, **Open Security**). Empty filtered views diagnose which filter token blocked traffic with quick reset/pause actions; clearing a session offers a **5-second Undo toast**; press `?` or `Cmd/Ctrl+/` for the shortcuts cheatsheet and `Cmd/Ctrl+Shift+P` for the Command Palette.
- **True Protocol Following & SOCKS5 Upstream**: Reads `ClientHello` ALPN offers and negotiates directly with origins for HTTP/2 multiplexing, WebSocket (RFC 6455), and SSE without HTTP/1.1 downgrades; upstream chaining supports HTTP/CONNECT, `socks5://`, and `socks5h://`.
- **Deep Inspection, AI SSE & Schema Decoding**: Pretty JSON + searchable JSON tree, GraphQL (`operationName` + query/variables), **LLM / AI SSE Stream Inspector** (OpenAI, Anthropic, Gemini thinking/output/tool calls/TTFT/tok/s), HTML/Markdown preview, XML, MessagePack, zero-dependency `.proto` schema decoding, inline JWT/timestamp/Base64 annotations, baseline Diff, and timing waterfalls.
- **17 Rule Types & Multi-Group Stacking**: Independently toggleable rule groups (`[[rule_groups]]`), terminal (`map-local`, `map-remote`, `break`, `break-res`, `block`), additive (`replace`, `json`, `delay`, `throttle`, `header`, `query`, `cors`, `no-cache`), host-level (`cert`, `client-cert`, `resolve`), and extension rules (`hook` mutate/preview, `proto`), with `{{uuid}}` / `{{hmac_sha256(...)}}` templates, GraphQL `op=` targeting, and an 8-slot FIFO breakpoint queue.
- **Diagrams, OpenAPI, Mock & Collaboration**: Interactive Waterfall, Swimlane Sequence, Dependency (`flowchart TD`), and Mermaid views; copy as cURL, `fetch`, Python, Rust, Go, TypeScript interfaces, or Rust `serde` structs; export OpenAPI 3.0.3, batch Local Mock directories, redacted HAR/HTML, or live `/s/{uuid}` & `--relay` shares.
- **AI-Native MCP & Web Console**: Built-in JSON-RPC 2.0 MCP server (`POST /mcp` and `zniff --mcp`, 10 tools) plus a WebAssembly console (`zniff --web`) with Basic auth, capture-port `allow_hosts` control (strict deny-all default in Docker), and verified in-place updates via `SHA256SUMS`.

---

## Screenshots

| Deep Inspection (GraphQL & JSON) | Filters & 17 Rewrite Rules |
| :---: | :---: |
| ![Deep Inspection](./docs/screenshots/02_detail_json_graphql.png) | ![Filters and Rules](./docs/screenshots/05_filter_rules_popup.png) |
| **Waterfall / Sequence Diagrams & Breakpoints** | **Adaptive Light & Dark Mode** |
| ![Waterfall and Sequence Diagrams](./docs/screenshots/07_sequence_diagram_waterfall.png) | ![Adaptive Light Mode](./docs/screenshots/10_theme_light_contrast.png) |

---

## Quick Start

### 1. Install

**macOS, Linux & Windows**
Download pre-built packages from [GitHub Releases](https://github.com/vicanso/zniff/releases): signed `.dmg` for macOS (Apple Silicon / Intel), `.tar.gz` for Linux (x86_64 / aarch64) and Windows (x86_64 / aarch64, contains `zniff.exe`). On Linux, run the bundled `install-linux.sh` from the unpacked tarball for a user-level install (launcher entry and autostart; `--no-autostart`, `--uninstall`, `--tag vX.Y.Z`). Package manager manifest templates for **Homebrew Cask**, **Scoop**, and **Winget** live in [`packaging/`](./packaging/README.md). Or build from source:
```bash
cargo install --git https://github.com/vicanso/zniff
# Or clone and package macOS .app:
# git clone https://github.com/vicanso/zniff.git && cd zniff && make bundle
```

**Docker (Web Console)**
```bash
docker run --rm -p 8080:8080 -p 8877:8877 \
  -e ZNIFF_ALLOW_HOSTS="*.vicanso.dev" \
  -v "$PWD/zniff-data:/data" \
  vicanso/zniff-web:latest
```
Open `http://127.0.0.1:8080` and click **Start** to bind capture port `8877`. The Docker image sets `ZNIFF_STRICT_ALLOW_HOSTS=1` by default: if `ZNIFF_ALLOW_HOSTS` is unset/empty and `/data/config.toml` has no entries, all proxied hosts are denied (`403`) until allowed via `ZNIFF_ALLOW_HOSTS` (read-only locked in UI) or in the Web console's **Access** footer popover (use `*` to allow all). The image also exposes `8879` for the optional relay (`--relay --token <secret>`). Use `:nightly` for the latest development build.

### 2. Configure Client & Root CA

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

---

## Migrate from Charles / Whistle in 30 Seconds

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

---

## Rewrite Rules

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

See the full syntax and examples in **[Rules & Configuration Guide](./docs/rules.md)** ([中文指南](./docs/rules-zh.md)), or browse the full **[Documentation Index](./docs/README.md)**.

---

## CLI Modes & MCP

| Mode | Command | Description |
| :--- | :--- | :--- |
| **Desktop GUI** | `zniff` | Native desktop window with in-process proxy engine. |
| **Web Console** | `zniff --web` | Headless HTTP server (`127.0.0.1:8080`) serving the Wasm UI; capture (`8877`) starts on demand. |
| **Web + Relay** | `zniff --web --relay --token <secret>` | Web console plus a relay on `0.0.0.0:8879` in one process (`--no-capture` for relay only). |
| **Relay Server** | `zniff --relay --token <secret>` | Standalone relay for real-time `/s/{uuid}` session sharing. |
| **MCP Bridge** | `zniff --mcp` | Stdio bridge for AI clients (also exposed over loopback `POST /mcp`). |

Global flags: `--data-dir <dir>` (env `ZNIFF_DATA_DIR`) overrides the config & CA directory; `--proxy-auth <user:pass>` protects the capture port; `--allow-hosts <hosts>` (`ZNIFF_ALLOW_HOSTS`) and `--strict-allow-hosts` (`ZNIFF_STRICT_ALLOW_HOSTS=1`, default in Docker) restrict proxied target domains; `--web-prefix` / `--relay-prefix` mount behind a reverse-proxy path; `zniff --help` lists everything, `zniff --version` prints the version.

<details>
<summary>Claude Desktop / Cursor MCP configuration & access flags</summary>

```json
{
  "mcpServers": {
    "zniff": {
      "command": "zniff",
      "args": ["--mcp", "--port", "8877"]
    }
  }
}
```

Available MCP tools (10): `list_flows`, `get_flow_detail`, `get_session_summary`, `annotate_flow`, `replay_flow`, `get_rules`, `set_rules`, `clear_flows`, `manage_breaks`, `generate_openapi`.

| Port | Control | Flag / Env |
| :--- | :--- | :--- |
| Console (`8080`) | HTTP Basic auth | `--web-auth <user:pass>` / `ZNIFF_WEB_AUTH` |
| Capture (`8877`) | `Proxy-Authorization` (407) | `--proxy-auth <user:pass>` / `ZNIFF_PROXY_AUTH` |
| Capture (`8877`) | Host allow-list (`403`, strict in Docker) | `--allow-hosts <hosts>` / `ZNIFF_ALLOW_HOSTS`, `--strict-allow-hosts` / `ZNIFF_STRICT_ALLOW_HOSTS` |
| Relay Upload | `Authorization: Bearer` | `--token <secret>` / `relay_token` |
| Relay View | HTTP Basic auth | `--view-auth <user:pass>` / `ZNIFF_RELAY_VIEW_AUTH` |

</details>

---

## Shortcuts, Storage & Development

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

**Data Directory**:
- **macOS**: `~/Library/Application Support/zniff`
- **Linux**: `$XDG_DATA_HOME/zniff` or `~/.local/share/zniff`
- **Windows**: `%APPDATA%\zniff`

Override with `--data-dir <dir>` or `ZNIFF_DATA_DIR` (the Docker image uses `/data`). Contains `config.toml`, `ca.crt` / `ca.key`, `proxy-snapshot.toml`, and `config.dev.toml` (dev mode, port `8878`). Sessions cap at 256 MB in memory, spilling unpinned bodies > 16 KB to a rotating 512 MB disk ring buffer (`zniff-spill/`).

**Build & Test**:
```bash
make dev      # Run desktop app in dev mode (port 8878)
make release  # Build release binary
make bundle   # Package macOS zniff.app
make web      # Run Web console (port 8080)
make test     # Run tests
make lint     # Run clippy
```

---

## License

[Apache License, Version 2.0](./LICENSE)
