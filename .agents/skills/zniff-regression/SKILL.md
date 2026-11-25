---
name: zniff-regression
description: >-
  Run zniff's 3-tier release and pre-merge regression workflow:
  (1) static/fmt/clippy/wasm32/unit & headless UI tests,
  (2) isolated live proxy & MCP smoke regression, and
  (3) optional desktop Retina screenshot inspection.
  Use when asked to verify a release, run a full regression, or validate major UI/proxy changes.
---

# zniff Regression Workflow

Run this 3-tier regression workflow in order. Stop and report immediately if any tier fails.

## Guardrails

- **Never** modify the user's real `config.toml`, `ca.crt`, or macOS/Linux system proxy settings during automated checks. Always use an isolated temporary `--data-dir` (or `ZNIFF_DEV=1` only when intentionally inspecting dev mode).
- **Never** run `git add` or `git commit` unless the user explicitly asks to commit.
- For any new domain-related tests or rules added during fixes, always use `vicanso.dev` subdomains (e.g. `api.vicanso.dev`, `test.vicanso.dev`).

---

## Tier 1: Static & Unit / Headless UI Gate (Always Run)

Run from the repository root:

```sh
make fmt && make lint && make check-wasm && make test
```

For feature-flag / minimal build confidence when `Cargo.toml`, `proxy`, `web`, or `updater` changed:

```sh
cargo check --manifest-path app/Cargo.toml --no-default-features
```

Expected: `0` clippy warnings, `wasm32` check passes, and all unit + headless UI tests pass.

- **Dirty-state UI transition coverage check**: Whenever changes touch UI state switches (flow selection/deselection, pagination, form/rule kind switching, popover/drawer dismiss), verify Tier 1 includes headless UI transition tests with pre-populated dirty state covering `A -> B` reset, `A -> A'` same-target preservation, dismiss paths (`Esc` / outside press / external input focus), and real UI control triggers (not just pure helper tests).

---

## Tier 2: Isolated Live Proxy & MCP Smoke (Always Run for Functional / Release Regression)

Execute the bundled smoke script, which launches `zniff --web` with a temporary `--data-dir` and ephemeral localhost ports (no system proxy changes):

```sh
bash .agents/skills/zniff-regression/scripts/smoke-proxy.sh
```

What this verifies end-to-end against a real `zniff` process:
- Local HTTP `GET` & `POST` capture through the live proxy
- MCP `list_flows`, `get_flow_detail`, and `replay_flow`
- MCP `set_rules` (`map-local` response override + `break` rule)
- Breakpoint pause + MCP `manage_breaks` (`list` + `release_all`)
- `/api/export.har` and `/api/export.html` exports
- MCP `clear_flows`

If internet access is available and you also want to exercise external HTTPS/H2/SSE/image/compression scenarios:

```sh
bash .agents/skills/zniff-regression/scripts/smoke-proxy.sh --with-external
```

---

## Tier 3: Desktop UI Retina Screenshot Inspection (Run on Major UI / Layout Changes or Release Sign-off)

Run when layout, theme, sidebar, toolbar, or detail cards changed:

1. Launch desktop `zniff` with an isolated temporary data dir (e.g., `ZNIFF_DATA_DIR=$(mktemp -d) cargo run --manifest-path app/Cargo.toml -- --port 18877`), without enabling system proxy.
2. Send a few sample requests (`scripts/test-traffic.sh --proxy http://127.0.0.1:18877 json status slow`) so list rows, badges, and detail cards are populated.
3. Capture Retina window screenshots (`screencapture -l <wid> -o`) and visually inspect:
   - Default `Split` layout (left facet sidebar + flow table + right summary/detail)
   - Collapsed sidebar (`<`/`>` toggle at footer left) falling back to top quick-search & drawer
   - `TopBottom` layout and Dark/Light theme + Zh/En labels (ensure sidebar quick chips fit on one row)
4. Clean up the temporary process and directory, and confirm `networksetup -getwebproxy Wi-Fi` (on macOS) still shows `Enabled: No`.

---

## Reporting Format

Report results by tier:
1. **Tier 1**: exact command, warning count, test pass/fail counts.
2. **Tier 2**: pass/fail of each smoke stage (`capture`, `MCP`, `map-local`, `break`, `HAR/HTML`, `clear`).
3. **Tier 3**: `Skipped (not requested)` or screenshot observations + confirmation that system proxy is untouched.
