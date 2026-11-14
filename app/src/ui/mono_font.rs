//! The monospaced font: the bundled JetBrains Mono NL, or the system's.
//!
//! The NL cut has no ligatures, so `!=`, `->` and `==` stay one column per
//! character in headers, JSON and hex. The four faces cover the weights the
//! UI draws mono text in (regular, medium, semibold, bold). `build.rs`
//! zlib-compresses them into `OUT_DIR` (~404 KB instead of ~838 KB, skipped on
//! headless `--no-default-features` server builds); at start each face is
//! inflated into a temporary `Vec<u8>`, handed to `register_font`, and freed.

use std::io::Read;

use flate2::read::ZlibDecoder;

#[allow(unused_imports)]
use super::*;

/// Zlib-compressed JetBrains Mono NL 2.304 faces built by `build.rs`
/// (SIL Open Font License 1.1, `assets/fonts/OFL.txt`).
#[cfg(any(feature = "desktop", target_arch = "wasm32"))]
const BUNDLED_FACES_ZZ: [&[u8]; 4] = [
    include_bytes!(concat!(env!("OUT_DIR"), "/JetBrainsMonoNL-Regular.ttf.zz")),
    include_bytes!(concat!(env!("OUT_DIR"), "/JetBrainsMonoNL-Medium.ttf.zz")),
    include_bytes!(concat!(env!("OUT_DIR"), "/JetBrainsMonoNL-SemiBold.ttf.zz")),
    include_bytes!(concat!(env!("OUT_DIR"), "/JetBrainsMonoNL-Bold.ttf.zz")),
];

#[cfg(not(any(feature = "desktop", target_arch = "wasm32")))]
const BUNDLED_FACES_ZZ: [&[u8]; 0] = [];

fn inflate_face(compressed: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut decoder = ZlibDecoder::new(compressed);
    let mut out = Vec::with_capacity(compressed.len() * 2);
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

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
        for compressed in BUNDLED_FACES_ZZ {
            let bytes = match inflate_face(compressed) {
                Ok(bytes) => bytes,
                Err(err) => {
                    log::warn!("解压内置等宽字体失败: {err}");
                    continue;
                }
            };
            match register_font(bytes).await {
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

    #[cfg(any(feature = "desktop", target_arch = "wasm32"))]
    #[test]
    fn bundled_faces_decompress_to_truetype() {
        for compressed in BUNDLED_FACES_ZZ {
            assert!(
                compressed.len() < 120_000,
                "compressed face should be shrunk: {} bytes",
                compressed.len()
            );
            let face = inflate_face(compressed).expect("valid zlib stream");
            assert!(face.len() > 200_000);
            assert_eq!(&face[..4], &[0, 1, 0, 0]);
        }
    }

    #[test]
    fn inflate_rejects_corrupt_stream() {
        assert!(inflate_face(b"not-zlib").is_err());
    }

    #[test]
    fn family_follows_choice() {
        let jb = Some(SharedString::from("JetBrains Mono NL"));
        assert_eq!(mono_family_for(false, jb.clone()), jb);
        assert_eq!(mono_family_for(true, jb), None);
        assert_eq!(mono_family_for(false, None), None);
    }
}
