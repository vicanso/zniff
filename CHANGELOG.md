## [0.5.1] - 2026-11-22

### 🚀 Features

- *(desktop)* Set zniff tray icon, menu item icons, and Clear Traffic action

### 🐛 Bug Fixes

- *(ui)* Allow copying response/request header and query values (fixes #30)
- *(docker)* Copy app/benches so Cargo manifest parses large_session bench
## [0.5.0] - 2026-11-22

### 🚀 Features

- *(ui)* Add actionable repair buttons for ERR flows (C1)
- *(ui)* Add actionable empty-filter state with token diagnosis (D1)
- *(ui)* Add 5-second undo toast for clear session (D5)
- *(ui)* Add built-in demo HAR and load sample traffic button (I3)
- *(ui)* Add keyboard shortcuts cheatsheet modal and command palette shortcuts (D4)
- *(ui)* Auto-scroll selected row into viewport on keyboard navigation (D3)
- *(ui)* Add theme mode, UI/mono font zoom, and CJK mono fallback (D6, D7, D8)
- *(ui)* Add filter prefix/value autocomplete and inline syntax error highlight (D2)
- *(ui)* Upgrade onboarding card with live 4-step setup checklist (C3)
- *(rules)* Record per-flow rule hit lines and display badges in list/detail (B1)
- *(ui)* Add contextual quick-rule actions from flow rows and detail menu (A4)
- *(ui)* Add batch TLS handshake failure banner at top of flow list (C2)
- *(rules)* Add per-line session hit counts and rule: filter token (B2)
- *(diagnostics)* Add redacted diagnostics bundle export (C4)
- *(ui)* Add tabbed Settings window and slim More menu into 3 groups (A2, A3)
- *(rules)* Add pre-save rule dry-run preview against current session (B3)
- *(ui)* Split Filter and Rules popovers and unify preset pill (A1)
- *(allow-hosts)* Track and display denied request summary with quick-allow
- *(ui)* Add built-in rule template library in rules panel (E4)
- *(mobile)* Add Android 7+ network_security_config and cert guidance (G2)
- *(copy)* Add HTTPie, PowerShell, axios, OkHttp, and URLSession copy formats (F1)
- *(mobile)* Add iOS certificate verification checklist and HTTPS probe (G3)
- *(ui)* Add network condition presets in toolbar and command palette (E3)
- *(ai)* Add redacted 'Copy for AI' Markdown diagnostic summary (F3)
- *(desktop)* Add system tray menu and opt-in minimize-to-tray (G1, ADR-23)
- *(export)* Add Postman v2.1, Bruno (.bru), and .http API collection exporters (F2)
- *(ui)* Add inline response mock editor with auto map-local rules (E1)
- *(session)* Add timeline session markers and mark: filter (E5)
- *(ui)* Auto-diff against source flow after replay and edit-resend (E2)
- *(diagnostics)* Add cold-start and auto-capture recovery timing instrumentation (H3)
- *(watch)* Add live watch alert rules, footer badge popover, and toast notifications (E6)
- *(mcp)* Add wait_for_flow, mock_flow, diff_flows tools and resources/prompts (F4)
- *(rules)* Add conditional breakpoints with header and body matching (E8, ADR-21)
- *(a11y)* Add keyboard focus rings and WCAG 2.1 AA theme contrast audit (H4)
- *(ui)* Open imported HAR/chlsj sessions as read-only tabs with cross-tab diff (E10)

### 🐛 Bug Fixes

- *(ui)* Replace reserved mod+m shortcut with mod+shift+m
- *(ui)* Ellipsize toolbar filter pill and show full filter on hover
- *(test)* Handle Windows CRLF line endings in on_key audit test

### 📚 Documentation

- Add v0.5 usability roadmap and ADR-17..23 decisions
- *(packaging)* Add Charles/Whistle migration guide and package manager templates (I2, I1)
- Mark v0.4.x usability optimizations as completed
- Mark v0.5 core usability optimizations as completed
- Mark remaining 19 v0.5.x features and ADR-21..23 as completed

### ⚡ Performance

- *(bench)* Add 10k/100k large-session benchmark suite and make bench target (H1)
- *(session)* Optimize facets, stamp-based row projection, and adaptive refresh throttling (H2)

### 🧪 Testing

- *(e2e)* Add comprehensive v0.5 real-process E2E functional test script
- *(ui)* Add headless functional UI test suite for v0.4.x & v0.5.x features
## [0.4.0] - 2026-11-21

### 🚀 Features

- *(launcher)* Add zero-config Chromium, proxied terminal, and Android adb capture
- *(rules)* Add dynamic template variables and HMAC-SHA256 request signing
- *(webhook)* Add loopback HTTP hook rule for custom request/response mutation
- *(openapi)* Add OpenAPI 3.0 & Mock dir export and generate_openapi MCP tool
- *(ui)* Add Hook tab, rule-assist roundtrip, and template var strip to quick rules
- *(launcher)* Show active ADB count with one-click reset and open selected flow origin in isolated browser
- *(openapi)* Add YAML export, one-click clipboard copy, and MCP yaml format
- *(webhook)* Add read-only mode=preview hook for detail body decryption
- *(proto)* Add zero-dependency .proto schema parser and gRPC/Protobuf detail decoder
- *(window)* Persist window position, mode, and display_id with multi-monitor fallback
- *(ui)* Add Cmd/Ctrl+Shift+P command palette powered by nsh_base::command
- Ship v0.4 onboarding, migration import, multi-group rules, SOCKS5/PAC, and LLM SSE inspector
- Resolve adb across GUI paths and refine right-pane onboarding layout
- Detect and reuse trusted local external root CAs
- Add Windows ARM64 build, test, and auto-update support
- Place request cards on left and response cards on right in top-bottom layout
- Add proxy allow-hosts control and strict deny-all mode for docker

### 🐛 Bug Fixes

- Keep single-domain sidebar counts reactive and polish empty/header UI

### 📚 Documentation

- Update document

### ⚡ Performance

- Yield paint frame and preserve OS folder in native file dialogs

### 🚜 Refactor

- *(proxy)* Split monolithic proxy.rs into cohesive submodules and tighten complexity guidelines

### 🧪 Testing

- Replace fixed sleeps with deterministic synchronization in proxy, web, and session tests

### ⚙️ Miscellaneous Tasks

- Upgrade nsh to 0.6.1
## [0.3.1] - 2026-11-18

### 🚀 Features

- *(ui)* Add rich About zniff modal with build metadata, links, and system info
- *(macos)* Route system menu About zniff to in-app About modal
- *(ui)* Keep QR entry in local-only mode with LAN switch prompt
- *(ui)* Toggle active filter pill instead of clearing and clean up update/about menu
- *(ui)* Open facet drawer by default when no primary filter is set
- *(ui)* Add auto and host+path display modes for the path column
- *(filter)* Use exact and wildcard matching for host/domain filters
- *(ui)* Add split, top-bottom, and 3-column layouts with grouped domain sidebar
- *(ui)* Upgrade to nsh 0.6.0, unify split sidebar layout, and polish proxy/MCP/UX

### 🐛 Bug Fixes

- *(updater)* Remove gitee fallback, fix truncated downloads, and show scrollable release notes
- *(ui)* Gate base_app_name on macOS and tests for Linux/Windows builds

### 🚜 Refactor

- *(ui)* Move proxy auth popover to footer beside listening controls
- *(ui)* Move raw filter expression input below structured filter fields

### ⚙️ Miscellaneous Tasks

- Version 0.2.1
- Version 0.3.0
## [0.3.0] - 2026-11-18

### 🚀 Features

- *(ui)* Add rich About zniff modal with build metadata, links, and system info
- *(macos)* Route system menu About zniff to in-app About modal
- *(ui)* Keep QR entry in local-only mode with LAN switch prompt
- *(ui)* Toggle active filter pill instead of clearing and clean up update/about menu
- *(ui)* Open facet drawer by default when no primary filter is set
- *(ui)* Add auto and host+path display modes for the path column
- *(filter)* Use exact and wildcard matching for host/domain filters
- *(ui)* Add split, top-bottom, and 3-column layouts with grouped domain sidebar
- *(ui)* Upgrade to nsh 0.6.0, unify split sidebar layout, and polish proxy/MCP/UX

### 🐛 Bug Fixes

- *(updater)* Remove gitee fallback, fix truncated downloads, and show scrollable release notes
- *(ui)* Gate base_app_name on macOS and tests for Linux/Windows builds

### 🚜 Refactor

- *(ui)* Move proxy auth popover to footer beside listening controls
- *(ui)* Move raw filter expression input below structured filter fields

### ⚙️ Miscellaneous Tasks

- Version 0.2.1
## [0.2.1] - 2026-11-15

### 🚀 Features

- *(config)* Isolate dev config and proxy snapshot from production
- *(ui)* Align column toggle menu order with table columns
- *(ui)* Support Path/Full URL toggle and Head/Middle/Tail ellipsize mode
- *(ui)* Adapt filter popover, rules editor, and left pane width to window size
- *(config)* Support SavedProfile TOML export and clipboard/drag-drop import
- *(ca)* Implement Windows and Linux certificate status check and removal
- *(ui,proxy)* Add default flow ID column and record aborted tunnel fallbacks
- *(ui,config)* Place ID column first and set default visible columns
- *(ui)* Use native file dialogs for HAR, HTML, profile, and response import/export
- *(ui)* Default exported profile filename to zniff-{name}.toml
- *(session)* Spill finished bodies over 32 KB to rotating temp segments
- *(session)* Lower inline body threshold to 16 KB and unlink spill segments on open
- *(share)* Redesign HTML share page to match zniff two-pane desktop UI
- *(share)* Incremental flows.json polling and lazy loading for spilled bodies
- *(ui)* Add TextView document preview toggle for HTML and Markdown bodies
- *(ui)* Persist and restore window size across launches
- Add client IP column and align column picker order with table
- *(ui)* Support reordering flow table columns via header drag and menu buttons
- *(list)* Support sorting request list by clicking column headers (#5)
- Implement P1 and P2 workflow, rule, and debugging enhancements
- Implement P3 batch replay, filtered summary, and CA trust diagnostics
- Add Mermaid call sequence diagram generator and refine summary toolbar
- Add swimlane sequence diagram and plot waterfall preview tabs
- Add critical-path waterfall and dependency stage views to sequence diagram panel
- Add throttle rule, message filters, concurrent batch replay, and default save dir (closes #4, #6, #7, #8)
- Add custom domain TLS cert mounting and in-place auto updater
- Redact sensitive data on HAR/HTML export and live share (closes #9)
- Add JSON-aware request and response body rewrite rules (closes #10)
- Match GraphQL rules and filters by operation name (closes #11)
- Support per-host client certificates for upstream mTLS (closes #12)
- Support directory mapping and upstream fallback in map-local (closes #13)
- Add connection phase timing breakdown and metadata
- Add Rust/Go code export and built-in MCP server
- Add no-cache and block semantic rules
- Add iOS Simulator CA installer and Linux system CA command
- Add collapsible domain and app filter facets
- Add secondary list filter, profile history, and memory optimizations
- Optimize sequence preview cap banner, breakpoint debounce, and ui details
- Keep live capture off the session lock and add a Linux installer
- Add a web server that shares the desktop UI and relays live shares
- Publish nightly multi-arch Docker images
- Add timing breakdown, MCP server, interactive viewer, and ICO preview
- Add proxy authentication, persistent data dir, and Windows CRLF test fix
- *(macos)* Integrate native title bar with top toolbar and add screenshot script
- *(web,relay)* Add combined web/relay modes, HTTP Basic auth, and headless Docker build
- *(ui,ca,web)* Add quick filters in facet drawer, per-host CA CN, and machine settings UI
- *(ui)* Bundle JetBrains Mono and size body editor by wrapped rows
- *(ui)* Add JSON tree view for request and response bodies
- *(rules)* Add custom host/IP resolve rule
- *(web)* Add detail loading states, state long polling, and unified error filter

### 🐛 Bug Fixes

- *(ui)* Keep raw hex bytes on image flows and update pane width on layout
- *(config,ca)* Sync profile switches on drop import and verify Windows cert before delstore
- *(ui)* Refine list column widths, method badge alignment, protocol localization, and compose card
- *(ui)* Strip query string when displaying path-only column
- *(present)* Keep Set-Cookie headers on a single row in pretty view
- *(ci)* Pin Linux builds to ubuntu-24.04 (glibc <= 2.39)
- *(proxy)* Prevent /favicon.ico and self-targeted requests from looping
- *(proxy)* Enable HTTP/1.1 connection upgrades for wss:// WebSocket relay (#2)
- *(breakpoint)* Apply edited response status code and close editor on Continue (#3)
- *(export)* Set initial save dialog directory and report cancelled exports (#4)
- Resolve cross-platform dead_code lint and update roadmap docs
- *(ci)* Ensure nightly release is always published (draft=false)
- Preserve unescaped Windows path backslashes in quoted rule tokens
- *(proxy)* Resolve issues #14, #15, #17, #26
- *(rules,config)* Resolve issues #16, #18, #19, #20, #23, #25
- *(ui,export)* Resolve issues #7, #21, #24, #27
- *(web)* Report config persistence failures on /api/hub and wasm UI (#25)
- *(build)* Compress bundled fonts via build.rs and copy assets in Dockerfile
- *(ui)* Clarify iOS CA install steps and balance QR modal spacing
- *(web)* Restrict wasm-opt to shipped features and throttle state polling
- *(ui)* Reserve desktop title bar side insets so Start/Stop stays clickable (#29)
- *(updater)* Remove gitee fallback, fix truncated downloads, and show scrollable release notes

### 📚 Documentation

- Sync AGENTS.md and known-issues.md with pending roadmap items
- Add README.md and README-zh.md
- Update document
- Streamline READMEs, track screenshots, and fix cargo fmt
- Remove redundant screenshot below demo video
- Fix install instructions in READMEs
- Update demo video link in READMEs
- Refine native & lightweight description in READMEs
- Update demo video
- Update README and rules for v0.2.0 and sync nsh.json version

### ⚡ Performance

- Shrink release/wasm binaries and optimize proxy, CA, web, and share hot paths
- *(web)* Upgrade to nsh 0.5.1 and optimize wasm bundle splitting

### 🚜 Refactor

- *(ui)* Split lib.rs into app/src/ui submodules and polish toolbar/scroll UX

### 🎨 Styling

- *(ui)* Thicken column resize handle indicator to 3px on hover and drag
- *(ui)* Align filter popover to right edge and reserve left list visibility
- *(ui)* Scale filter popover width with window and left list width
- *(ui)* Rebalance default flow column widths to fit 560px pane
- *(ui)* Widen filter popover to cover 30% of left flow table
- *(ui)* Right-align gap, elapsed, and size columns in flow table
- *(ui)* Drop leading plus sign from flow gap column

### ⚙️ Miscellaneous Tasks

- *(scripts)* Add test-traffic.sh for public endpoint curl scenarios
- *(scripts)* Add explicit gzip endpoints and gzip alias in test-traffic.sh
- *(nightly)* Add rolling nightly pre-release workflow
- *(build)* Add make nightly target to trigger nightly pre-release
- Add linux-aarch64 build target to release and nightly workflows
- Upgrade nsh to 0.5.0
- *(scripts)* Record real-time window video during screenshot capture
- *(scripts,docs)* Add bilingual intro card to demo video and update READMEs
- Version 0.2.0

### 💼 Other

- Add right padding in scrollable containers and rightmost table column
## [0.2.0] - 2026-11-15

### 🚀 Features

- *(config)* Isolate dev config and proxy snapshot from production
- *(ui)* Align column toggle menu order with table columns
- *(ui)* Support Path/Full URL toggle and Head/Middle/Tail ellipsize mode
- *(ui)* Adapt filter popover, rules editor, and left pane width to window size
- *(config)* Support SavedProfile TOML export and clipboard/drag-drop import
- *(ca)* Implement Windows and Linux certificate status check and removal
- *(ui,proxy)* Add default flow ID column and record aborted tunnel fallbacks
- *(ui,config)* Place ID column first and set default visible columns
- *(ui)* Use native file dialogs for HAR, HTML, profile, and response import/export
- *(ui)* Default exported profile filename to zniff-{name}.toml
- *(session)* Spill finished bodies over 32 KB to rotating temp segments
- *(session)* Lower inline body threshold to 16 KB and unlink spill segments on open
- *(share)* Redesign HTML share page to match zniff two-pane desktop UI
- *(share)* Incremental flows.json polling and lazy loading for spilled bodies
- *(ui)* Add TextView document preview toggle for HTML and Markdown bodies
- *(ui)* Persist and restore window size across launches
- Add client IP column and align column picker order with table
- *(ui)* Support reordering flow table columns via header drag and menu buttons
- *(list)* Support sorting request list by clicking column headers (#5)
- Implement P1 and P2 workflow, rule, and debugging enhancements
- Implement P3 batch replay, filtered summary, and CA trust diagnostics
- Add Mermaid call sequence diagram generator and refine summary toolbar
- Add swimlane sequence diagram and plot waterfall preview tabs
- Add critical-path waterfall and dependency stage views to sequence diagram panel
- Add throttle rule, message filters, concurrent batch replay, and default save dir (closes #4, #6, #7, #8)
- Add custom domain TLS cert mounting and in-place auto updater
- Redact sensitive data on HAR/HTML export and live share (closes #9)
- Add JSON-aware request and response body rewrite rules (closes #10)
- Match GraphQL rules and filters by operation name (closes #11)
- Support per-host client certificates for upstream mTLS (closes #12)
- Support directory mapping and upstream fallback in map-local (closes #13)
- Add connection phase timing breakdown and metadata
- Add Rust/Go code export and built-in MCP server
- Add no-cache and block semantic rules
- Add iOS Simulator CA installer and Linux system CA command
- Add collapsible domain and app filter facets
- Add secondary list filter, profile history, and memory optimizations
- Optimize sequence preview cap banner, breakpoint debounce, and ui details
- Keep live capture off the session lock and add a Linux installer
- Add a web server that shares the desktop UI and relays live shares
- Publish nightly multi-arch Docker images
- Add timing breakdown, MCP server, interactive viewer, and ICO preview
- Add proxy authentication, persistent data dir, and Windows CRLF test fix
- *(macos)* Integrate native title bar with top toolbar and add screenshot script
- *(web,relay)* Add combined web/relay modes, HTTP Basic auth, and headless Docker build
- *(ui,ca,web)* Add quick filters in facet drawer, per-host CA CN, and machine settings UI
- *(ui)* Bundle JetBrains Mono and size body editor by wrapped rows
- *(ui)* Add JSON tree view for request and response bodies
- *(rules)* Add custom host/IP resolve rule
- *(web)* Add detail loading states, state long polling, and unified error filter

### 🐛 Bug Fixes

- *(ui)* Keep raw hex bytes on image flows and update pane width on layout
- *(config,ca)* Sync profile switches on drop import and verify Windows cert before delstore
- *(ui)* Refine list column widths, method badge alignment, protocol localization, and compose card
- *(ui)* Strip query string when displaying path-only column
- *(present)* Keep Set-Cookie headers on a single row in pretty view
- *(ci)* Pin Linux builds to ubuntu-24.04 (glibc <= 2.39)
- *(proxy)* Prevent /favicon.ico and self-targeted requests from looping
- *(proxy)* Enable HTTP/1.1 connection upgrades for wss:// WebSocket relay (#2)
- *(breakpoint)* Apply edited response status code and close editor on Continue (#3)
- *(export)* Set initial save dialog directory and report cancelled exports (#4)
- Resolve cross-platform dead_code lint and update roadmap docs
- *(ci)* Ensure nightly release is always published (draft=false)
- Preserve unescaped Windows path backslashes in quoted rule tokens
- *(proxy)* Resolve issues #14, #15, #17, #26
- *(rules,config)* Resolve issues #16, #18, #19, #20, #23, #25
- *(ui,export)* Resolve issues #7, #21, #24, #27
- *(web)* Report config persistence failures on /api/hub and wasm UI (#25)
- *(build)* Compress bundled fonts via build.rs and copy assets in Dockerfile
- *(ui)* Clarify iOS CA install steps and balance QR modal spacing
- *(web)* Restrict wasm-opt to shipped features and throttle state polling
- *(ui)* Reserve desktop title bar side insets so Start/Stop stays clickable (#29)

### 📚 Documentation

- Sync AGENTS.md and known-issues.md with pending roadmap items
- Add README.md and README-zh.md
- Update document
- Streamline READMEs, track screenshots, and fix cargo fmt
- Remove redundant screenshot below demo video
- Fix install instructions in READMEs
- Update demo video link in READMEs
- Refine native & lightweight description in READMEs
- Update demo video
- Update README and rules for v0.2.0 and sync nsh.json version

### ⚡ Performance

- Shrink release/wasm binaries and optimize proxy, CA, web, and share hot paths
- *(web)* Upgrade to nsh 0.5.1 and optimize wasm bundle splitting

### 🚜 Refactor

- *(ui)* Split lib.rs into app/src/ui submodules and polish toolbar/scroll UX

### 🎨 Styling

- *(ui)* Thicken column resize handle indicator to 3px on hover and drag
- *(ui)* Align filter popover to right edge and reserve left list visibility
- *(ui)* Scale filter popover width with window and left list width
- *(ui)* Rebalance default flow column widths to fit 560px pane
- *(ui)* Widen filter popover to cover 30% of left flow table
- *(ui)* Right-align gap, elapsed, and size columns in flow table
- *(ui)* Drop leading plus sign from flow gap column

### ⚙️ Miscellaneous Tasks

- *(scripts)* Add test-traffic.sh for public endpoint curl scenarios
- *(scripts)* Add explicit gzip endpoints and gzip alias in test-traffic.sh
- *(nightly)* Add rolling nightly pre-release workflow
- *(build)* Add make nightly target to trigger nightly pre-release
- Add linux-aarch64 build target to release and nightly workflows
- Upgrade nsh to 0.5.0
- *(scripts)* Record real-time window video during screenshot capture
- *(scripts,docs)* Add bilingual intro card to demo video and update READMEs

### 💼 Other

- Add right padding in scrollable containers and rightmost table column
## [unreleased]

### 🚀 Features

- Add the zniff desktop capture app
- Add session tools, prefs, and the two-pane capture UI
- Add rewrite rules, flow tools, and a denser capture list
- Show the list time as a gap from the previous flow
- Toggle pretty and raw on the body card
- Split Cookie and Set-Cookie in pretty mode
- Sort headers alphabetically in pretty mode
- Resize the list and choose its columns
- Clarify failed rows, fold detail cards, and move HAR import
- Show the session count and put Clear first
- Show bodies and diffs in the nsh editor
- Number hex dump rows
- Remember whether bodies start pretty
- Highlight the pane splitter on hover
- Open column settings from the header
- Put the path beside the method
- Replace text inside a captured body
- Open row actions from a right click
- Edit rules with line numbers and a quick add row
- Show form bodies as annotated rows
- Show which rules rewrote a flow
- Step through flows and replay from the keyboard
- Filter the list from chips under the search box
- Export only the flows the filter is showing
- Mute rules or breakpoints without editing the rule text
- Wrap the macOS release in an app with the icon
- Share filtered captures and replace the flow list
- Unify filter and rules config panel with domain and path-prefix OR support
- Add header/CORS quick rules, live syntax validation, OR-aware focus/mute, and update docs
- Add always-visible save bar and custom names for saved filters and rules
- Save filter, rules, and upstream as one named profile
- *(ui)* Replace saved profile list with bottom dropdown selector and refine filter config dialog

### 🐛 Bug Fixes

- Finish HTTPS bodies when hyper stops polling
- Draw the host column in the monospace font
- Keep image previews proportional and quiet
- Let the path column take the leftover width
- Redraw the open detail when the pretty default changes
- Highlight JSON that arrives as plain text
- Put every detail title inside its card
- Keep the star dot when the host column is hidden
- Separate the body editor from the note above it
- Import Chrome HAR entries marked http/2.0
- Move the column picker up and stop rebuilding row menus
- Resolve linux and windows clippy lints in ca and sysproxy
- Avoid unused snapshot variable on windows in apply_snoop

### 📚 Documentation

- Skip restoring the window frame
- Skip restoring the window frame on nsh 0.3

### 🎨 Styling

- Polish filter and rules panel layout, rule type tabs, and badges

### 🧪 Testing

- Prove the install page and that a share keeps its filter
- Keep domain OR fixtures on distinct hosts
- Use generic hosts and paths in rule examples

### ⚙️ Miscellaneous Tasks

- Upgrade nsh to 0.3 and the capture crates
- Hide test-only helpers from the library build
- Watch the app with bacon and bump versions from app/Cargo.toml
- Test and publish zniff from app/Cargo.toml
- Fix clippy lints, format code, and require make fmt/lint in AGENTS.md
- Sign, package as dmg, and notarize macOS release builds
- Upgrade nsh to 0.4.0 and use menu popup for saved profiles
