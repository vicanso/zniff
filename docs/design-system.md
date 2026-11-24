# zniff Design System

Normative UI standards for `zniff`, following the [NSH Design Guides](https://nsh.dev/docs/design-guides.md). All desktop (`macOS`, `Linux`, `Windows`) and Web (`wasm32`) views compose from `app/src/ui/kit/` using semantic tokens from `app/src/ui/theme.rs`.

## Visual Language

- **Quiet, precise, high-density workspace**: content (traffic list, inspector payloads, rules) carries the screen; borders, fills, and accent colors stay restrained.
- **One primary focal point per surface**: reserve `ButtonVariant::Primary` for the primary commit or capture trigger; use `Ghost`, `Secondary`, or `Outline` for routine toolbar and inline actions.
- **No nested cards**: separate groups inside a panel with section titles, spacing, or single hairlines (`ui_divider()`) rather than stacking bordered cards inside cards.

## Finite Scales

### Typography (`app/src/ui/kit/tokens.rs`)

| Token | Size (pt) | Role |
| --- | --- | --- |
| `TEXT_XS` | `11.0` | Micro badges, compact counts, secondary metadata |
| `TEXT_SM` | `12.0` | Default desktop body, controls, table rows, inputs |
| `TEXT_MD` | `13.0` | Section titles and grouped headers |
| `TEXT_LG` | `15.0` | Modal and sheet titles |

- Use the UI font for prose/labels; reserve monospace (`mono(...)`) for URLs, headers, code, filters, shortcuts, and tabular numbers (`.tabular_nums()`).

### Radius (`app/src/ui/kit/tokens.rs`)

| Token | Value (pt) | Role |
| --- | --- | --- |
| `RADIUS_SM` | `4.0` | Badges, tags, inline code chips, compact indicators |
| `RADIUS_MD` | `6.0` | Buttons, inputs, menu rows, tabs (matches `Theme::radius`) |
| `RADIUS_LG` | `10.0` | Popovers, menus, dialogs, modal surfaces |

- Nested corners follow concentric geometry: `inner ≈ max(0, outer − gap)`. Use `.rounded_full()` only for status dots and circular indicators.

### Control Sizes (`ControlSize`)

| Size | Height | Horizontal Padding | Text Size | Icon Size | Radius | Use |
| --- | --- | --- | --- | --- | --- | --- |
| `Xs` | `22.0` | `8.0` (`px_2`) | `11.0` | `12.0` | `4.0` | Status footer triggers, compact badges/pills |
| `Sm` | `28.0` | `10.0` | `12.0` | `14.0` | `6.0` | Desktop toolbars, inline controls, form rows (default) |
| `Md` | `32.0` | `12.0` (`px_3`) | `12.0` | `16.0` | `6.0` | Dialog commits, prominent form actions |

- Icon-only buttons (`ui_icon_button`) are square (`height × height`) and must carry an accessible `.label(...)` plus a tooltip when non-obvious.

## Color Tokens (`app/src/ui/theme.rs`)

- Every surface, text, border, status, scrim, and timing color is defined once as an `AdaptiveColor` function in `app/src/ui/theme.rs` and synced into `nsh_base::Theme` via `build_nsh_theme(...)`.
- Feature UI files must never contain raw hex, `rgb()`, `rgba()`, or `white()`/`black()` literals.
- Semantic text/background pairs are verified against WCAG 2.1 AA (`>= 4.5:1` text, `>= 3.0:1` focus/UI components) in `app/src/ui/a11y.rs`.

## Interaction, Pointers & Accessibility

- **Pointer cursors**: controls use the default arrow cursor; reserve `Cursor::Pointer` exclusively for external URLs/links that leave the app.
- **Keyboard focus**: every interactive control applies `.focus_visible(apply_focus_ring)` (`2pt` high-contrast ring, `2pt` offset outside the border).
- **Overlays**: every `Popover` and `Menu` shares `ui_popup_style` (and `ui_popup_starting_style`); Escape closes the topmost layer and returns focus logically.

## Motion Tokens & Inventory

| Token | Duration | Usage | Reduced Motion |
| --- | --- | --- | --- |
| `MOTION_INSTANT_MS` | `120.0` | Button/row hover & press color washes | Instant swap |
| `MOTION_FAST_MS` | `160.0` | Popover/menu/toast reveal & dismiss | No fade/translate (`ui_popup_starting_style` returns identity) |
| `MOTION_BASE_MS` | `220.0` | Panel / tab transitions & state swaps | Instant swap |

### Baseline Motion Inventory

| State Change | Tool & Tokens | Reduced Motion (`reduce_motion()`) |
| --- | --- | --- |
| Button / toggle hover & press | `transition_colors()`, `MOTION_INSTANT_MS`, ease-out | Instant color switch |
| Popover / Menu open & close | `popup_style(ui_popup_style)` + `popup_starting_style(ui_popup_starting_style)` (`MOTION_FAST_MS`) | Appears/disappears immediately without fade |
| Toast arrive & dismiss | `ToastMotion::sonner()`, `transition_all()`, `MOTION_FAST_MS` | Instant show/hide |
| Virtualized table rows & counters | No motion (high-frequency updates stay calm) | None |
