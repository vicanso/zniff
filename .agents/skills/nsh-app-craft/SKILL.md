---
name: nsh-app-craft
description: >-
  Portable engineering, UX, Apple-grade motion, performance, and cross-platform
  (desktop + wasm32 + companion mobile HTML) standards for building and auditing
  Native Shell (NSH) Rust applications. Activate when building, reviewing,
  refactoring, or polishing NSH apps, UI kits, responsive layouts, animations,
  or dual-target desktop/web builds.
---

# NSH App Craft

Cross-project engineering playbook for building responsive, calm, high-craft **NSH (Native Shell)** Rust applications across desktop (`macOS` / `Linux` / `Windows`), browser WebAssembly (`wasm32-unknown-unknown`), headless CLI modes (`--no-default-features`), and companion HTTP/HTML mobile pages.

This skill complements `nsh` (API/build recipes) and `nsh-design-guides` (visual design tokens). Read the relevant reference file on demand instead of guessing ad-hoc timings or workarounds.

## Core Architecture & Quality Gates

1. **NSH-First Component Reuse & Design Alignment**
   - Before building or refactoring any UI control, layout primitive, or overlay, check local `nsh-base` / `nsh-core` source or `https://nsh.dev/elements/{name}.md` first and reuse native components (`button`, `tabs`/`tab`, `select`, `sheet`, `resize_handle`, `collapsible`/`accordion`, `checkbox`, `radio_group`/`radio`, `switch`, `link`, `popover`, `menu`, `tooltip`, `data_table`, `command`, etc.) rather than hand-rolling `view().role(...)` or custom drag/overlay engines.
   - Align layout hierarchy, spacing/density, keyboard roving navigation, focus-visible rings, accessibility semantics, and reduced-motion-aware animations with `nsh-design-guides` and official NSH coding/design guides before layering app-specific tokens.

2. **Structural Complexity Caps**
   - Split new panels, modals, popovers, and domains into single-responsibility modules.
   - **File length**: target `<= 600` lines, soft cap `800`, hard cap `1200` (excluding `#[cfg(test)]`; extract `tests.rs` once tests exceed `400` lines).
   - **Function length**: target `<= 80` lines, hard cap `120`; nesting depth `<= 4` (use early return, `let-else`, `?`, and small helpers); parameters `<= 7` (bundle larger signatures into a `Props` / options struct; never add `#[allow(clippy::too_many_arguments)]`).
   - Do not append new subsystems to legacy oversized files; opportunistically extract cohesive blocks when editing them.

2. **Centralized UI Kit (`src/ui/kit/`)**
   - Centralize spacing, radii, typography, durations (`MOTION_*`), press scales (`PRESS_SCALE*`), buttons, switches, modal shells, popup styles, rotating chevrons, and pulse animations in one kit module.
   - Downstream screens consume kit primitives instead of hardcoding `duration(...)`, ad-hoc easing arrays, or duplicate variant styling.

3. **Verification Gates (After Every Change)**
   - **Formatting**: `cargo fmt --check`
   - **Native clippy**: `cargo clippy --all-targets -- -D warnings`
   - **Wasm32 lint & build**: `cargo clippy --target wasm32-unknown-unknown -- -D warnings` (catches lints inside `#[cfg(target_arch = "wasm32")]` branches that native clippy never sees)
   - **Headless feature check** (if app supports CLI/server without GUI): `cargo check --no-default-features`
   - **Tests**: `cargo test` including headless NSH mount/interaction unit tests across locales and viewports

## Reference Manuals (Progressive Disclosure)

Load only the manual(s) relevant to your task:

- **UX & Copy Audit**: [UX Principles & Guardrails](./references/ux-principles.md) — system autonomy, truthful state reflection, progressive disclosure, terminology/punctuation hygiene, draft buffers, and 2-step destructive confirmations.
- **State-Change Motion & Responsiveness**: [Motion System & Mobile Standards](./references/motion-system.md) — Apple-grade timing/easing tokens, `reduce_motion()`, popup enter/exit triples, modal motion, hover-intent debounce, `computed(...)` memoization, and companion HTML/CSS mobile rules.
- **NSH 0.6 & Dual-Target Pitfalls**: [NSH & Wasm32 Pitfalls](./references/nsh-pitfalls.md) — component lifecycle ("runs once"), `DataTable` cell disposal rules, nested `scroll()` / `content_culling` pointer guards, `ui_zoom` parity, narrow-window test isolation, and `wasm32` forbidden APIs.
