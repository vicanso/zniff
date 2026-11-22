# CLI, Web Console & MCP

English | [简体中文](./cli-web-mcp-zh.md)

zniff runs as a desktop app by default, and can also run headless as a web console, a relay for live sharing, or an MCP bridge for AI clients. Back to the [User Guide](./guide.md).

## Run Modes

| Mode | Command | Description |
| :--- | :--- | :--- |
| **Desktop GUI** | `zniff` | Native desktop window with in-process proxy engine. |
| **Web Console** | `zniff --web` | Headless HTTP server (`127.0.0.1:8080`) serving the Wasm UI; capture (`8877`) starts on demand. |
| **Web + Relay** | `zniff --web --relay --token <secret>` | Web console plus a relay on `0.0.0.0:8879` in one process (`--no-capture` for relay only). |
| **Relay Server** | `zniff --relay --token <secret>` | Standalone relay for real-time `/s/{uuid}` session sharing. |
| **MCP Bridge** | `zniff --mcp` | Stdio bridge for AI clients (also exposed over loopback `POST /mcp`). |

The web console runs the same UI compiled to WebAssembly; the browser talks to the `zniff --web` process over `/api` and never listens, decrypts, or changes the system proxy itself. Only clients that set the capture port as their HTTP proxy are recorded.

## Global Flags

- `--data-dir <dir>` (env `ZNIFF_DATA_DIR`) overrides the config & CA directory.
- `--proxy-auth <user:pass>` protects the capture port.
- `--allow-hosts <hosts>` (`ZNIFF_ALLOW_HOSTS`) and `--strict-allow-hosts` (`ZNIFF_STRICT_ALLOW_HOSTS=1`, default in Docker) restrict proxied target domains.
- `--web-prefix` / `--relay-prefix` mount behind a reverse-proxy path.
- `zniff --help` lists everything, `zniff --version` prints the version.

## Docker

```bash
docker run --rm -p 8080:8080 -p 8877:8877 \
  -e ZNIFF_ALLOW_HOSTS="*.vicanso.dev" \
  -v "$PWD/zniff-data:/data" \
  vicanso/zniff-web:latest
```

Open `http://127.0.0.1:8080` and click **Start** to bind capture port `8877`. The Docker image sets `ZNIFF_STRICT_ALLOW_HOSTS=1` by default: if `ZNIFF_ALLOW_HOSTS` is unset/empty and `/data/config.toml` has no entries, all proxied hosts are denied (`403`) until allowed via `ZNIFF_ALLOW_HOSTS` (read-only locked in UI) or in the Web console's **Access** footer popover (use `*` to allow all). The image also exposes `8879` for the optional relay (`--relay --token <secret>`). Use `:nightly` for the latest development build.

## Access Control

| Port | Control | Flag / Env |
| :--- | :--- | :--- |
| Console (`8080`) | HTTP Basic auth | `--web-auth <user:pass>` / `ZNIFF_WEB_AUTH` |
| Capture (`8877`) | `Proxy-Authorization` (407) | `--proxy-auth <user:pass>` / `ZNIFF_PROXY_AUTH` |
| Capture (`8877`) | Host allow-list (`403`, strict in Docker) | `--allow-hosts <hosts>` / `ZNIFF_ALLOW_HOSTS`, `--strict-allow-hosts` / `ZNIFF_STRICT_ALLOW_HOSTS` |
| Relay Upload | `Authorization: Bearer` | `--token <secret>` / `relay_token` |
| Relay View | HTTP Basic auth | `--view-auth <user:pass>` / `ZNIFF_RELAY_VIEW_AUTH` |

## MCP

zniff ships a JSON-RPC 2.0 MCP server, available over loopback `POST /mcp` on the capture port and as a stdio bridge via `zniff --mcp`.

Claude Desktop / Cursor configuration:

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

**Tools (10)**: `list_flows`, `get_flow_detail`, `get_session_summary`, `annotate_flow`, `replay_flow`, `get_rules`, `set_rules`, `clear_flows`, `manage_breaks`, `generate_openapi`.

**Resources**: `zniff://flows/current` (summary and statistics of captured flows in the active session) and `zniff://rules/active` (active rules, rule groups, TLS bypass list, and switch states).

**Prompts**: `analyze_failed_flow` — analyzes a failed or slow flow using redacted request/response diagnostics (optional `flow_id`; defaults to the most recent failed or slow flow).
