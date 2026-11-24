//! Keyboard accessibility helpers and WCAG 2.1 contrast-ratio audit (H4).
//!
//! Provides a consistent high-contrast focus ring for keyboard Tab navigation
//! (`apply_focus_ring`) and unit-tested WCAG 2.1 AA contrast verification for
//! all semantic foreground/background pairs in [`crate::ui::theme`].

#[allow(unused_imports)]
use super::*;

/// Applies a high-contrast 2px keyboard focus ring (`:focus-visible`) to interactive controls.
pub(crate) fn apply_focus_ring<S: Styled>(style: S) -> S {
    style
        .outline(px(2.0))
        .outline_color(focus_ring())
        .outline_offset(px(2.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn srgb_channel(byte: u8) -> f64 {
        let c = f64::from(byte) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    fn relative_luminance(rgb: u32) -> f64 {
        let r = srgb_channel(((rgb >> 16) & 0xff) as u8);
        let g = srgb_channel(((rgb >> 8) & 0xff) as u8);
        let b = srgb_channel((rgb & 0xff) as u8);
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    fn contrast_ratio(fg: u32, bg: u32) -> f64 {
        let l1 = relative_luminance(fg);
        let l2 = relative_luminance(bg);
        let (hi, lo) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }

    fn pair_rgb(c: AdaptiveColor) -> (u32, u32) {
        let to_u32 = |hsla: Hsla| -> u32 {
            let rgba: Rgba = hsla.into();
            let r = (rgba.r.clamp(0.0, 1.0) * 255.0).round() as u32;
            let g = (rgba.g.clamp(0.0, 1.0) * 255.0).round() as u32;
            let b = (rgba.b.clamp(0.0, 1.0) * 255.0).round() as u32;
            (r << 16) | (g << 8) | b
        };
        (to_u32(c.light), to_u32(c.dark))
    }

    #[test]
    fn wcag_contrast_math_matches_reference_values() {
        assert!((contrast_ratio(0x000000, 0xffffff) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(0xffffff, 0xffffff) - 1.0).abs() < 0.001);
    }

    #[test]
    fn all_theme_text_and_focus_pairs_meet_wcag_aa() {
        let text_pairs: &[(&str, AdaptiveColor, AdaptiveColor, f64)] = &[
            ("fg_primary on bg_canvas", fg_primary(), bg_canvas(), 4.5),
            ("fg_primary on bg_card", fg_primary(), bg_card(), 4.5),
            ("fg_primary on bg_control", fg_primary(), bg_control(), 4.5),
            (
                "fg_secondary on bg_canvas",
                fg_secondary(),
                bg_canvas(),
                4.5,
            ),
            ("fg_secondary on bg_card", fg_secondary(), bg_card(), 4.5),
            (
                "fg_secondary on bg_control",
                fg_secondary(),
                bg_control(),
                4.5,
            ),
            ("fg_muted on bg_canvas", fg_muted(), bg_canvas(), 4.5),
            ("fg_muted on bg_card", fg_muted(), bg_card(), 4.5),
            ("fg_muted on bg_control", fg_muted(), bg_control(), 4.5),
            (
                "fg_muted on bg_control_hover",
                fg_muted(),
                bg_control_hover(),
                4.5,
            ),
            (
                "fg_muted on bg_control_active",
                fg_muted(),
                bg_control_active(),
                4.5,
            ),
            (
                "accent_blue_text on bg_card",
                accent_blue_text(),
                bg_card(),
                4.5,
            ),
            (
                "accent_blue_text on accent_blue_soft",
                accent_blue_text(),
                accent_blue_soft(),
                4.5,
            ),
            (
                "fg_on_accent on accent_blue",
                fg_on_accent(),
                accent_blue(),
                4.5,
            ),
            (
                "fg_on_accent on accent_blue_hover",
                fg_on_accent(),
                accent_blue_hover(),
                4.5,
            ),
            (
                "fg_on_accent on accent_blue_active",
                fg_on_accent(),
                accent_blue_active(),
                4.5,
            ),
            (
                "fg_on_accent on danger_button_bg",
                fg_on_accent(),
                danger_button_bg(),
                4.5,
            ),
            (
                "fg_on_accent on danger_button_hover",
                fg_on_accent(),
                danger_button_hover(),
                4.5,
            ),
            (
                "fg_on_accent on danger_button_active",
                fg_on_accent(),
                danger_button_active(),
                4.5,
            ),
            (
                "fg_primary on bg_control_active",
                fg_primary(),
                bg_control_active(),
                4.5,
            ),
            (
                "status_ok_fg on status_ok_bg",
                status_ok_fg(),
                status_ok_bg(),
                4.5,
            ),
            (
                "status_warn_fg on status_warn_bg",
                status_warn_fg(),
                status_warn_bg(),
                4.5,
            ),
            (
                "status_err_fg on status_err_bg",
                status_err_fg(),
                status_err_bg(),
                4.5,
            ),
            ("method GET", method_fg("GET"), method_bg("GET"), 4.5),
            ("method GET on bg_card", method_fg("GET"), bg_card(), 4.5),
            (
                "method GET on bg_control_hover",
                method_fg("GET"),
                bg_control_hover(),
                4.5,
            ),
            ("method POST", method_fg("POST"), method_bg("POST"), 4.5),
            ("method POST on bg_card", method_fg("POST"), bg_card(), 4.5),
            (
                "method POST on bg_control_hover",
                method_fg("POST"),
                bg_control_hover(),
                4.5,
            ),
            ("method PUT", method_fg("PUT"), method_bg("PUT"), 4.5),
            ("method PUT on bg_card", method_fg("PUT"), bg_card(), 4.5),
            (
                "method PUT on bg_control_hover",
                method_fg("PUT"),
                bg_control_hover(),
                4.5,
            ),
            (
                "method DELETE",
                method_fg("DELETE"),
                method_bg("DELETE"),
                4.5,
            ),
            (
                "method DELETE on bg_card",
                method_fg("DELETE"),
                bg_card(),
                4.5,
            ),
            (
                "method DELETE on bg_control_hover",
                method_fg("DELETE"),
                bg_control_hover(),
                4.5,
            ),
            (
                "method CONNECT",
                method_fg("CONNECT"),
                method_bg("CONNECT"),
                4.5,
            ),
            (
                "method CONNECT on bg_card",
                method_fg("CONNECT"),
                bg_card(),
                4.5,
            ),
            (
                "method CONNECT on bg_control_hover",
                method_fg("CONNECT"),
                bg_control_hover(),
                4.5,
            ),
            ("json_string on bg_code", json_string_fg(), bg_code(), 4.5),
            ("json_number on bg_code", json_number_fg(), bg_code(), 4.5),
            ("json_literal on bg_code", json_literal_fg(), bg_code(), 4.5),
            // WCAG 2.1 SC 1.4.11 Non-text UI component contrast >= 3.0:1
            ("focus_ring on bg_canvas", focus_ring(), bg_canvas(), 3.0),
            ("focus_ring on bg_chrome", focus_ring(), bg_chrome(), 3.0),
            ("focus_ring on bg_card", focus_ring(), bg_card(), 3.0),
            ("focus_ring on bg_control", focus_ring(), bg_control(), 3.0),
        ];

        let mut failures = Vec::new();
        for &(name, fg, bg, min_ratio) in text_pairs {
            let (fg_l, fg_d) = pair_rgb(fg);
            let (bg_l, bg_d) = pair_rgb(bg);
            let light_ratio = contrast_ratio(fg_l, bg_l);
            let dark_ratio = contrast_ratio(fg_d, bg_d);
            if light_ratio < min_ratio {
                failures.push(format!(
                    "{name} [light]: {light_ratio:.2}:1 < {min_ratio:.1}:1 (#{fg_l:06x} on #{bg_l:06x})"
                ));
            }
            if dark_ratio < min_ratio {
                failures.push(format!(
                    "{name} [dark]: {dark_ratio:.2}:1 < {min_ratio:.1}:1 (#{fg_d:06x} on #{bg_d:06x})"
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "WCAG AA contrast failures:\n{}",
            failures.join("\n")
        );
    }
}
