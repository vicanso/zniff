//! The monospaced font: the bundled JetBrains Mono NL, or the system's.
//!
//! The NL cut has no ligatures, so `!=`, `->` and `==` stay one column per
//! character in headers, JSON and hex. The four faces cover the weights the
//! UI draws mono text in (regular, medium, semibold, bold). They are
//! registered once at start; the theme's `font-mono` token then names the
//! family, so `mono()`, `editor()` and `diff_view()` all follow it.

#[allow(unused_imports)]
use super::*;

/// JetBrains Mono NL 2.304 (SIL Open Font License 1.1, `assets/fonts/OFL.txt`).
const BUNDLED_FACES: [&[u8]; 4] = [
    include_bytes!("../../assets/fonts/JetBrainsMonoNL-Regular.ttf"),
    include_bytes!("../../assets/fonts/JetBrainsMonoNL-Medium.ttf"),
    include_bytes!("../../assets/fonts/JetBrainsMonoNL-SemiBold.ttf"),
    include_bytes!("../../assets/fonts/JetBrainsMonoNL-Bold.ttf"),
];

/// The user's monospaced font choice and the bundled family once registered.
#[derive(Clone, Copy)]
pub(crate) struct MonoFont {
    /// True draws mono text in the system's monospaced font.
    pub(crate) system: Signal<bool>,
    /// The bundled family name, `None` until registered (or when it failed).
    bundled: Signal<Option<SharedString>>,
}

/// Registers the bundled faces and keeps the theme's `font-mono` on the
/// current choice. Called once from `App`.
pub(crate) fn provide_mono_font(system: bool) -> MonoFont {
    let font = MonoFont {
        system: signal(system),
        bundled: signal(None),
    };
    provide_context(font);
    spawn_detached(async move {
        let mut family = None;
        for face in BUNDLED_FACES {
            match register_font(face).await {
                Ok(name) => {
                    family.get_or_insert(name);
                }
                Err(err) => log::warn!("注册内置等宽字体失败: {err}"),
            }
        }
        font.bundled.set(family.map(SharedString::from));
    });
    // Skips the first run when nothing would change, so a system-font start
    // does not replace the tokens it was given.
    let applied = Rc::new(Cell::new(false));
    effect(move || {
        let family = mono_family_for(font.system.get(), font.bundled.get());
        if family.is_none() && !applied.get() {
            return;
        }
        applied.set(true);
        let mut theme = nsh_base::Theme::default();
        if let Some(family) = family {
            theme.set_font(nsh_core::element::FONT_MONO, family.as_str());
        }
        set_tokens(theme.into_tokens());
    });
    font
}

/// The family `font-mono` should name: the bundled one unless the user
/// picked the system's or it has not registered.
pub(crate) fn mono_family_for(system: bool, bundled: Option<SharedString>) -> Option<SharedString> {
    if system { None } else { bundled }
}

/// The app's [`MonoFont`].
pub(crate) fn mono_font() -> MonoFont {
    use_context().expect("mono font context")
}

/// Switches the monospaced font and writes the choice to `config.toml`.
pub(crate) fn set_system_mono_font(controls: &Controls, system: bool) {
    let font = mono_font();
    if font.system.get_untracked() == system {
        return;
    }
    font.system.set(system);
    save_config_or_notify(controls, PrefName::MonoFont, |config| {
        config.system_mono_font = system;
    });
}

/// The 「等宽字体」 row in the 「更多」 menu: `[JetBrains Mono][系统]`.
pub(crate) fn menu_mono_font_row(controls: Arc<Controls>) -> impl IntoElement {
    let font = mono_font();
    let c_bundled = controls.clone();
    let c_system = controls;
    view()
        .h(px(32.0))
        .px_2p5()
        .gap_2()
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .child(
            control_text(tr("等宽字体", "Mono font"))
                .flex_1()
                .text_color(fg_secondary()),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(menu_inline_pill(
                    "JetBrains Mono",
                    move || !font.system.get(),
                    move || set_system_mono_font(&c_bundled, false),
                ))
                .child(menu_inline_pill(
                    tr("系统", "System"),
                    move || font.system.get(),
                    move || set_system_mono_font(&c_system, true),
                )),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_faces_are_truetype() {
        for face in BUNDLED_FACES {
            assert!(face.len() > 100_000);
            assert_eq!(&face[..4], &[0, 1, 0, 0]);
        }
    }

    #[test]
    fn family_follows_choice() {
        let jb = Some(SharedString::from("JetBrains Mono NL"));
        assert_eq!(mono_family_for(false, jb.clone()), jb);
        assert_eq!(mono_family_for(true, jb), None);
        assert_eq!(mono_family_for(false, None), None);
    }
}
