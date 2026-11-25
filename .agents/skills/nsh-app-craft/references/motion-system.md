# State-Change Motion System & Companion Web/Mobile Rules

Audit UI motion by **state change**—appear, disappear, move, resize, content swap, icon rotate, number tick, press, hover, loading, live pulse, and error/success—never as isolated widget decoration.

## 1. Unified Timing & Scale Tokens (`ui/kit/tokens.rs`)

Strictly ordered tokens keep every surface in one calm cadence:

```rust
pub(crate) const MOTION_PRESS_MS: f32 = 90.0;
pub(crate) const MOTION_EXIT_MS: f32 = 110.0;
pub(crate) const MOTION_INSTANT_MS: f32 = 120.0;
pub(crate) const MOTION_FAST_MS: f32 = 160.0;
pub(crate) const MOTION_BASE_MS: f32 = 220.0;
pub(crate) const MOTION_PULSE_MS: f32 = 1400.0;
pub(crate) const PRESS_SCALE: f32 = 0.97;
pub(crate) const PRESS_SCALE_ICON: f32 = 0.94;

pub(crate) fn motion_ms(ms: f32) -> f32 {
    if nsh_core::prelude::reduce_motion() { 0.0 } else { ms }
}
```

- **Deceleration curve (enter / hover / toggle)**: `ease_bezier(0.22, 1.0, 0.36, 1.0)`.
- **Acceleration curve (exit)**: `ease_bezier(0.32, 0.0, 0.67, 1.0)` with `MOTION_EXIT_MS` (faster out than in).
- **Reactive duration**: Always bind `.duration(move || motion_ms(...))` instead of static `.duration(MOTION_*)` so toggling OS "Reduce motion" takes effect live.

## 2. Standard UI Kit Motion Primitives

- **Interactive buttons**: Use `.transition()` (transitioning `COLORS | OPACITY | SHADOW | TRANSFORM`—note `.transition_colors()` alone ignores `scale`!), `.duration(move || motion_ms(MOTION_INSTANT_MS))`, and `.active(|s| ... .duration(move || motion_ms(MOTION_PRESS_MS)).scale(move || if reduce_motion() { 1.0_f32 } else { PRESS_SCALE }))`.
- **Popovers, Menus & HoverCards**: Always pair all three hooks:
  - `.popup_style(ui_popup_style)` (`transition()` + `motion_ms(MOTION_FAST_MS)`)
  - `.popup_starting_style(ui_popup_starting_style)` (`s.opacity(0.0).scale(0.97)` unless `reduce_motion()`)
  - `.popup_ending_style(ui_popup_ending_style)` (`s.opacity(0.0).scale(0.97).duration(MOTION_EXIT_MS).ease_bezier(0.32, 0.0, 0.67, 1.0)` unless `reduce_motion()`)
- **Modals**: Attach `motion().entering(...)` to both `ui_modal_backdrop` (opacity fade) and `ui_modal_card` (opacity + `scale(0.97 -> 1.0)` + `translate_y(4 -> 0)` gated by `!reduce_motion()`).
- **Switches & Rotating Chevrons**: Gate both track color and thumb transform on `motion_ms(MOTION_INSTANT_MS)`. Use `ui_rotating_chevron` (`0° -> 90°` via `.rotate(...)` + `.transition_transform()`) for collapsible sections rather than snapping icons.
- **Scoped Layout Reflow**: For sidebar collapse or panel splits, trigger `ElementRef::animate_layout(LayoutTransition::new(LayoutCurve::bezier(0.22, 1.0, 0.36, 1.0), MOTION_FAST_MS))` inside the press handler right before toggling state—only when `!reduce_motion()`—so lists and siblings glide smoothly without animating unrelated updates.
- **Calm Live Indicators**: Share one `KeyframeAnimation::new(Keyframes::Pulse).frame_rate(60.0)` helper; attach only while active AND `!reduce_motion()`.
- **Scroll Reset**: When switching tabs inside a shared outer `scroll()`, call `scroll_ref.scroll_to_offset(px(0.0), false)` in an `effect`.

## 3. High-Frame-Rate Performance Rules

- **Hover-intent debounce**: Tooltip overlays over dense virtualized tables use a `280ms` `set_timeout` on cold entry (`Stored<Option<Spawned<()>>>`), updating immediately once active and cancelling on `on_mouse_leave` so fast pointer sweeps cause zero flicker.
- **Zero-jitter inline hover controls**: Reserve fixed-size slots (e.g., `22×22px`) and fade opacity (`0.0_f32 -> 1.0_f32`) instead of mounting/unmounting buttons with `when(hovered)`.
- **Memoized derived views**: Wrap non-trivial filtering, parsing, or grouping (`filtered_apps`, `collect_rule_lines`, preview formatting, checklist summaries) in `computed(move || ...)` at component setup and borrow via `.with(...)`.

## 4. Companion Mobile / Web HTML Pages

Whenever an NSH app serves companion mobile/LAN pages (install guides, snapshots, live shares):
- **Adaptive colors**: Define `:root { color-scheme: dark light; ... }` CSS variables with `@media (prefers-color-scheme: light)`.
- **Viewport & notch safe areas**: Use `<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">` and `max(..., env(safe-area-inset-*))` padding; use `height: 100vh; height: 100dvh;` on full-screen mobile shells.
- **Touch targets & wrap**: Minimum `34–46px` tap targets (`@media (pointer: coarse)`), `touch-action: manipulation`, wrapping toolbars / horizontally scrollable pill rows, and stacked modal bodies under `@media (max-width: 860px)`.
- **Plain-HTTP LAN copy fallback**: Never rely solely on `navigator.clipboard.writeText` (unavailable on insecure `http://<lan-ip>` origins); always fall back cleanly to a hidden `<textarea>` + `document.execCommand("copy")` with in-place `"Copied"` button state.
- **Reduced motion in CSS**: Always include `@media (prefers-reduced-motion: reduce) { *, *::before, *::after { transition: none !important; animation: none !important; } }`.
