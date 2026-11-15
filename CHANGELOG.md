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
