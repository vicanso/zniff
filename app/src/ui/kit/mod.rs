//! Reusable styled UI component kit layered on top of `nsh_base` and `theme`.

mod button;
mod surface;
mod tokens;

#[allow(unused_imports)]
pub(crate) use self::button::*;
#[allow(unused_imports)]
pub(crate) use self::surface::*;
#[allow(unused_imports)]
pub(crate) use self::tokens::*;

#[cfg(test)]
mod tests {
    use nsh_core::prelude::px;

    use super::*;
    use crate::ui::theme::{
        accent_blue, bg_canvas, bg_card, build_nsh_theme, build_nsh_theme_with_fonts,
        danger_button_bg, fg_on_accent, fg_primary, focus_ring,
    };

    #[test]
    fn scale_tokens_are_strictly_ordered() {
        const {
            assert!(TEXT_XS < TEXT_SM);
            assert!(TEXT_SM < TEXT_MD);
            assert!(TEXT_MD < TEXT_LG);
            assert!(RADIUS_SM < RADIUS_MD);
            assert!(RADIUS_MD < RADIUS_LG);
            assert!(MOTION_PRESS_MS < MOTION_EXIT_MS);
            assert!(MOTION_EXIT_MS < MOTION_INSTANT_MS);
            assert!(MOTION_INSTANT_MS < MOTION_FAST_MS);
            assert!(MOTION_FAST_MS < MOTION_BASE_MS);
            assert!(MOTION_BASE_MS < MOTION_PULSE_MS);
            assert!(PRESS_SCALE_ICON > 0.85 && PRESS_SCALE_ICON < PRESS_SCALE);
            assert!(PRESS_SCALE < 1.0);
        }

        let sizes = [ControlSize::Xs, ControlSize::Sm, ControlSize::Md];
        for pair in sizes.windows(2) {
            let [smaller, larger] = [pair[0], pair[1]];
            assert!(smaller.height() < larger.height());
            assert!(smaller.icon_size() < larger.icon_size());
            assert!(smaller.padding_x() < larger.padding_x());
        }
    }

    #[test]
    fn build_nsh_theme_syncs_semantic_tokens_and_mono_font() {
        let base_theme = build_nsh_theme(None);
        assert_eq!(base_theme.background, bg_canvas());
        assert_eq!(base_theme.foreground, fg_primary());
        assert_eq!(base_theme.card, bg_card());
        assert_eq!(base_theme.primary, accent_blue());
        assert_eq!(base_theme.primary_foreground, fg_on_accent());
        assert_eq!(base_theme.destructive, danger_button_bg());
        assert_eq!(base_theme.ring, focus_ring());
        assert_eq!(base_theme.radius, px(RADIUS_MD));
        assert_eq!(base_theme.font_mono, None);

        let mono_theme = build_nsh_theme(Some("Menlo"));
        assert_eq!(mono_theme.font_sans, None);
        assert_eq!(mono_theme.font_mono.as_deref(), Some("Menlo"));
        assert_eq!(mono_theme.radius, px(RADIUS_MD));

        let both_theme = build_nsh_theme_with_fonts(Some("Inter"), Some("Fira Code"));
        assert_eq!(both_theme.font_sans.as_deref(), Some("Inter"));
        assert_eq!(both_theme.font_mono.as_deref(), Some("Fira Code"));
    }
}
