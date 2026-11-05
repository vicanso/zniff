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
