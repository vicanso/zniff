# NSH 0.6 & Dual-Target (`desktop` + `wasm32`) Pitfalls

Battle-tested framework constraints for NSH 0.6.1 Rust UI code:

## 1. Reactive Closures & Component Lifetime
- NSH components **run once**; only prop/style/control-flow bindings (`text`, `when`, `each`, `dynamic`, `style_with`) re-run.
- `tr(zh, en)` returns `impl Fn() -> String + Copy`. When mixing `tr(...)` branches with formatted `String`s inside `.label(...)` or `text(...)`, annotate `move || -> SharedString` or `move || -> String` consistently.
- `Text::font_weight` takes a static `FontWeight`, NOT a closure.
- `View` uses `.label(...)` for accessibility, whereas `nsh_base::ButtonElement` uses `.accessibility_label(...)`.
- In closures returning `f32` for `.scale(...)` or `.opacity(...)`, explicitly write `1.0_f32` / `0.0_f32` to avoid Rust's `float-literal-f32-fallback` clippy error (`-D warnings`).
- `KeyframeTiming` in NSH 0.6 is `#[non_exhaustive]`: construct via `let mut t = KeyframeTiming::default(); t.duration_ms = ...;`, never struct-update syntax `..KeyframeTiming::default()`.
- Always use `.self_start()` on `View` (not `.align_self_start()`).
- Use `.transition()` (not `.transition_all()` unless shape radii also animate; note `.transition_colors()` overwrites earlier `.transition_transform()` and vice-versa—only `.transition()` covers colors + opacity + shadow + transform together).

## 2. `DataTable` Constraints
- Never place `tooltip()`, `context_menu()`, or state/props owned by external row lifetimes directly inside recycled table cells—recycling throws `panic: a prop read or written after its owner was disposed`. Keep floating overlays (`CellHover`, context menu) beside the table and push hover coordinates/text into signals.
- Never wrap `DataTable` in `context_menu`: it shrinks row height to `0`.
- `DataTable::font_size` is `Prop<Points>` (`IntoReactive<Points>`), so `.font_size(move || scaled_px(12.0))` keeps cell text synced with UI zoom alongside `.row_height(...)`.
- `nsh-base 0.6.1` treats `.selection(None)` on `DataTable` as "uncontrolled / unset" (leaving the previous row highlight intact); to clear selection programmatically, return `Some(DataTableSelection::default())` (whose `row: None, column: None` resets selection) and call `api.select(None)` when `selected` becomes `None` or disappears from filtered rows.

## 3. Nested Scroll & Content Culling (`nsh 0.6`)
- Never attach `.flex_1()` to a `scroll()` inside an auto-height popover/menu/drawer (where `.w_full()` + `.max_h(...)` is used instead of a fixed height): `flex-basis: 0px` collapses the scroll area to `0px`.
- Always constrain popovers, drawers, and modals with `window()`-bounded `.max_w(...)` / `.max_h(...)` and an inner `scroll()`.
- Inside outer `scroll()` containers hosting `editor()`, `diff_view()`, or inner `scroll()`/`virtual_list()`:
  - Wrap outer scroll content root in `uncull_nested_scroll(view())`.
  - Wrap every `editor()` and `diff_view()` container in `guard_culled_pointer_events(view())` so delayed native view mounting does not trap wheel/click events on outer `Layer::Paint`.

## 4. Responsive Auto-Collapse & Headless Test Isolation
- Perform narrow-window sidebar auto-collapse (`window_w < threshold`) in the **shell / splitter / quick-bar wiring**, NOT inside the standalone sidebar component or toolbar toggle widget—otherwise headless component tests mounting widgets in small `300×400` harnesses fail or hide their controls unexpectedly.
- Avoid writing temporary resize-induced collapse back into persistent user config (`config.toml`), so widening the window restores the user's expanded layout cleanly.

## 5. `ui_zoom` & `reduce_motion()`
- Apply `scaled_px(...)` consistently to `Text`, `DataTable`, and editable/read-only `Editor` `.font_size(...)` & `.line_height(...)`—never mix `Points(12.0)` constants on one editor while scaling others.
- Note `nsh_core::prelude::reduce_motion()` reads `env(|e| e.reduce_motion)`, which requires an active NSH runtime environment; keep pure non-UI constant unit tests free of `reduce_motion()` calls.

## 6. `wasm32-unknown-unknown` & `--no-default-features`
- Never call `std::env::temp_dir()`, `std::process::id()`, `SystemTime::now()`, or `Instant::now()` on `wasm32` (runtime panic); abstract wall-clock timestamps behind a cross-platform helper (e.g., `clock::now()`).
- Always run `cargo clippy --target wasm32-unknown-unknown -- -D warnings` after touching shared `ui/` code—clean native clippy routinely hides `needless_return`, `collapsible_if`, or `single_match` errors inside `#[cfg(target_arch = "wasm32")]` paths.

## 7. Packaging Binary Size (`nsh package` & `[profile.release]`)
- `nsh package` builds desktop targets from a generated workspace (`app/.nsh/rust/Cargo.toml`). Cargo only reads `[profile.*]` from the root workspace manifest, so `[profile.release]` in `app/Cargo.toml` is ignored—leaving binaries unstripped, with full unwind tables and no LTO (often inflating a ~25 MB desktop binary to ~65–70 MB, or ~135 MB for macOS universal).
- Mirror `[profile.release]` (`opt-level = "s"`, `codegen-units = 1`, `lto = true`, `strip = true`, `panic = "abort"`) in `app/.cargo/config.toml`. Cargo walks upward from `app/.nsh/rust/` and applies it automatically to manual `nsh package`, `Makefile`, and macOS/Linux/Windows CI jobs without affecting sibling crates outside `app/` or `[profile.web]` (which explicitly sets its own profile keys).
- Omit target-conditional `[profile.release.package.<name>]` overrides from `app/.cargo/config.toml` when those crates are excluded on `wasm32`, otherwise `wasm32` builds emit unused-package-spec warnings.
- For pure Rust apps (`"root"` in `nsh.json`), `nsh 0.6.1` hardcodes all optional platform features in generated `rust-*/Cargo.toml` manifests; enabling `lto = true` in `app/.cargo/config.toml` allows the linker to dead-strip unused optional subsystems (`quickjs`, `tree_sitter`, `emoji_picker`, etc.).
