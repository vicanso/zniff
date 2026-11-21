//! Theme mode (Light / Dark / System) and UI/mono font zoom (`80%..=150%`).

#[allow(unused_imports)]
use super::*;

use nsh_core::platform::Appearance;

#[derive(Clone, Copy)]
pub(crate) struct AppearancePrefs {
    pub(crate) theme_mode: Signal<config::ThemeMode>,
    pub(crate) ui_zoom: Signal<u16>,
}

pub(crate) fn nsh_appearance(mode: config::ThemeMode) -> Appearance {
    match mode {
        config::ThemeMode::System => Appearance::System,
        config::ThemeMode::Light => Appearance::Light,
        config::ThemeMode::Dark => Appearance::Dark,
    }
}

pub(crate) fn provide_appearance_prefs(
    theme_mode: config::ThemeMode,
    ui_zoom: u16,
) -> AppearancePrefs {
    let prefs = AppearancePrefs {
        theme_mode: signal(theme_mode),
        ui_zoom: signal(config::clamp_ui_zoom(ui_zoom)),
    };
    provide_context(prefs);
    effect(move || {
        set_color_scheme(nsh_appearance(prefs.theme_mode.get()));
    });
    prefs
}

pub(crate) fn appearance_prefs() -> Option<AppearancePrefs> {
    use_context::<AppearancePrefs>()
}

pub(crate) fn ui_zoom_scale() -> f32 {
    appearance_prefs()
        .map(|prefs| f32::from(prefs.ui_zoom.get()) / 100.0)
        .unwrap_or(1.0)
}

pub(crate) fn scaled_px(base: f32) -> Points {
    px((base * ui_zoom_scale() * 2.0).round() / 2.0)
}

pub(crate) fn next_zoom_percent(current: u16, delta: i16) -> u16 {
    let base = config::clamp_ui_zoom(current) as i32;
    let next = (base + i32::from(delta)).clamp(
        i32::from(config::UI_ZOOM_MIN),
        i32::from(config::UI_ZOOM_MAX),
    );
    next as u16
}

pub(crate) fn set_theme_mode(controls: &Controls, mode: config::ThemeMode) {
    let Some(prefs) = appearance_prefs() else {
        return;
    };
    if prefs.theme_mode.get_untracked() == mode {
        return;
    }
    prefs.theme_mode.set(mode);
    save_config_or_notify(controls, PrefName::Theme, |cfg| {
        cfg.theme_mode = mode;
    });
}

pub(crate) fn step_ui_zoom(controls: &Controls, delta: i16) {
    let Some(prefs) = appearance_prefs() else {
        return;
    };
    let current = prefs.ui_zoom.get_untracked();
    let next = next_zoom_percent(current, delta);
    if next == current {
        set_status(controls, StatusMsg::UiZoomChanged(next));
        return;
    }
    prefs.ui_zoom.set(next);
    set_status(controls, StatusMsg::UiZoomChanged(next));
    save_config_or_notify(controls, PrefName::UiZoom, |cfg| {
        cfg.ui_zoom = next;
    });
}

pub(crate) fn reset_ui_zoom(controls: &Controls) {
    let Some(prefs) = appearance_prefs() else {
        return;
    };
    let next = config::UI_ZOOM_DEFAULT;
    if prefs.ui_zoom.get_untracked() != next {
        prefs.ui_zoom.set(next);
        save_config_or_notify(controls, PrefName::UiZoom, |cfg| {
            cfg.ui_zoom = next;
        });
    }
    set_status(controls, StatusMsg::UiZoomChanged(next));
}

/// The 「主题」 row in the 「更多」 menu: `[系统][浅色][深色]`.
pub(crate) fn menu_theme_row(controls: Arc<Controls>) -> impl IntoElement {
    let Some(prefs) = appearance_prefs() else {
        return view();
    };
    let c_sys = controls.clone();
    let c_light = controls.clone();
    let c_dark = controls;
    view()
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .child(
            control_text(tr("主题", "Theme"))
                .flex_1()
                .text_color(fg_secondary()),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(menu_inline_pill(
                    tr("系统", "System"),
                    move || prefs.theme_mode.get() == config::ThemeMode::System,
                    move || set_theme_mode(&c_sys, config::ThemeMode::System),
                ))
                .child(menu_inline_pill(
                    tr("浅色", "Light"),
                    move || prefs.theme_mode.get() == config::ThemeMode::Light,
                    move || set_theme_mode(&c_light, config::ThemeMode::Light),
                ))
                .child(menu_inline_pill(
                    tr("深色", "Dark"),
                    move || prefs.theme_mode.get() == config::ThemeMode::Dark,
                    move || set_theme_mode(&c_dark, config::ThemeMode::Dark),
                )),
        )
}

/// The 「缩放」 row in the 「更多」 menu: `[-][100%][+]`.
pub(crate) fn menu_zoom_row(controls: Arc<Controls>) -> impl IntoElement {
    let Some(prefs) = appearance_prefs() else {
        return view();
    };
    let c_out = controls.clone();
    let c_reset = controls.clone();
    let c_in = controls;
    view()
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .child(
            control_text(tr("缩放", "Zoom"))
                .flex_1()
                .text_color(fg_secondary()),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(menu_inline_pill(
                    "−",
                    || false,
                    move || step_ui_zoom(&c_out, -(config::UI_ZOOM_STEP as i16)),
                ))
                .child(menu_inline_pill(
                    move || SharedString::from(format!("{}%", prefs.ui_zoom.get())),
                    move || prefs.ui_zoom.get() != config::UI_ZOOM_DEFAULT,
                    move || reset_ui_zoom(&c_reset),
                ))
                .child(menu_inline_pill(
                    "+",
                    || false,
                    move || step_ui_zoom(&c_in, config::UI_ZOOM_STEP as i16),
                )),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_mode_maps_to_nsh_appearance() {
        assert_eq!(
            nsh_appearance(config::ThemeMode::System),
            Appearance::System
        );
        assert_eq!(nsh_appearance(config::ThemeMode::Light), Appearance::Light);
        assert_eq!(nsh_appearance(config::ThemeMode::Dark), Appearance::Dark);
    }

    #[test]
    fn zoom_steps_and_clamps_within_bounds() {
        assert_eq!(next_zoom_percent(100, 10), 110);
        assert_eq!(next_zoom_percent(100, -10), 90);
        assert_eq!(next_zoom_percent(150, 10), 150);
        assert_eq!(next_zoom_percent(80, -10), 80);
        assert_eq!(next_zoom_percent(0, 10), 110);
    }
}
