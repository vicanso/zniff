//! The monospaced font: the bundled JetBrains Mono NL, or the system's.
//!
//! The NL cut has no ligatures, so `!=`, `->` and `==` stay one column per
//! character in headers, JSON and hex. The four faces cover the weights the
//! UI draws mono text in (regular, medium, semibold, bold). Native desktop
//! builds inflate zlib-compressed faces embedded by `build.rs`; `wasm32`
//! builds lazily fetch hashed `fonts/*.ttf` assets when bundled mode is active.

#[cfg(not(target_arch = "wasm32"))]
use std::io::Read;

#[cfg(not(target_arch = "wasm32"))]
use flate2::read::ZlibDecoder;

#[allow(unused_imports)]
use super::*;

/// Zlib-compressed JetBrains Mono NL 2.304 faces built by `build.rs`
/// (SIL Open Font License 1.1, `assets/fonts/OFL.txt`).
#[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
const BUNDLED_FACES_ZZ: [&[u8]; 4] = [
    include_bytes!(concat!(env!("OUT_DIR"), "/JetBrainsMonoNL-Regular.ttf.zz")),
    include_bytes!(concat!(env!("OUT_DIR"), "/JetBrainsMonoNL-Medium.ttf.zz")),
    include_bytes!(concat!(env!("OUT_DIR"), "/JetBrainsMonoNL-SemiBold.ttf.zz")),
    include_bytes!(concat!(env!("OUT_DIR"), "/JetBrainsMonoNL-Bold.ttf.zz")),
];

#[cfg(all(not(feature = "desktop"), not(target_arch = "wasm32")))]
const BUNDLED_FACES_ZZ: [&[u8]; 0] = [];

#[cfg(target_arch = "wasm32")]
const WEB_FONT_REL_PATHS: [&str; 4] = [
    env!("ZNIFF_MONO_REGULAR"),
    env!("ZNIFF_MONO_MEDIUM"),
    env!("ZNIFF_MONO_SEMIBOLD"),
    env!("ZNIFF_MONO_BOLD"),
];

#[cfg(not(target_arch = "wasm32"))]
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
    #[cfg(not(target_arch = "wasm32"))]
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
    #[cfg(target_arch = "wasm32")]
    {
        let loading = Rc::new(Cell::new(false));
        let loading_eff = Rc::clone(&loading);
        effect(move || {
            if font.system.get() || font.bundled.get().is_some() || loading_eff.get() {
                return;
            }
            loading_eff.set(true);
            let loading_task = Rc::clone(&loading_eff);
            spawn_detached(async move {
                let mut family = None;
                for rel in WEB_FONT_REL_PATHS {
                    let resp = match fetch(HttpRequest::get(rel)).await {
                        Ok(resp) => resp,
                        Err(err) => {
                            log::warn!("加载内置等宽字体失败 ({rel}): {err}");
                            continue;
                        }
                    };
                    if !resp.ok() {
                        log::warn!("加载内置等宽字体失败 ({rel}): HTTP {}", resp.status());
                        continue;
                    }
                    match register_font(resp.body).await {
                        Ok(name) => {
                            family.get_or_insert(name);
                        }
                        Err(err) => log::warn!("注册内置等宽字体失败 ({rel}): {err}"),
                    }
                }
                if let Some(name) = family {
                    font.bundled.set(Some(SharedString::from(name)));
                } else {
                    loading_task.set(false);
                }
            });
        });
    }
    let langs = ui_langs();
    effect(move || {
        let lang = ui_lang(langs);
        let family = mono_family_for_lang(font.system.get(), font.bundled.get(), lang);
        set_tokens(build_nsh_theme(family.as_deref()).into_tokens());
    });
    font
}

/// Platform-appropriate CJK monospace fallback list used when the UI is in Chinese.
pub(crate) fn cjk_mono_fallback() -> &'static str {
    #[cfg(target_arch = "wasm32")]
    {
        "PingFang SC, Microsoft YaHei, Noto Sans Mono CJK SC, ui-monospace, monospace"
    }
    #[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
    {
        "PingFang SC, Hiragino Sans GB, Menlo, monospace"
    }
    #[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
    {
        "Cascadia Mono, Consolas, Microsoft YaHei UI, Microsoft YaHei, monospace"
    }
    #[cfg(all(
        not(target_arch = "wasm32"),
        not(target_os = "macos"),
        not(target_os = "windows")
    ))]
    {
        "Noto Sans Mono CJK SC, Sarasa Mono SC, WenQuanYi Micro Hei Mono, DejaVu Sans Mono, monospace"
    }
}

/// System monospace stack with CJK fallback when Chinese UI uses the system mono font.
fn system_cjk_mono_stack() -> &'static str {
    #[cfg(target_arch = "wasm32")]
    {
        "ui-monospace, SFMono-Regular, Menlo, PingFang SC, Microsoft YaHei, Noto Sans Mono CJK SC, monospace"
    }
    #[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
    {
        "Menlo, SF Mono, PingFang SC, Hiragino Sans GB, monospace"
    }
    #[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
    {
        "Cascadia Mono, Consolas, Microsoft YaHei UI, Microsoft YaHei, monospace"
    }
    #[cfg(all(
        not(target_arch = "wasm32"),
        not(target_os = "macos"),
        not(target_os = "windows")
    ))]
    {
        "DejaVu Sans Mono, Noto Sans Mono CJK SC, Sarasa Mono SC, WenQuanYi Micro Hei Mono, monospace"
    }
}

/// The family `font-mono` should name: the bundled one unless the user
/// picked the system's or it has not registered.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn mono_family_for(system: bool, bundled: Option<SharedString>) -> Option<SharedString> {
    mono_family_for_lang(system, bundled, Lang::En)
}

/// Language-aware `font-mono` family. In Chinese UI, appends CJK monospace
/// fallbacks so mixed Latin + Chinese table/detail cells stay crisp and aligned.
pub(crate) fn mono_family_for_lang(
    system: bool,
    bundled: Option<SharedString>,
    lang: Lang,
) -> Option<SharedString> {
    let base = if system { None } else { bundled };
    match lang {
        Lang::En => base,
        Lang::Zh => match base {
            Some(family) => Some(SharedString::from(format!(
                "{}, {}",
                family.as_str(),
                cjk_mono_fallback()
            ))),
            None => Some(SharedString::from(system_cjk_mono_stack())),
        },
    }
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
            radio_group()
                .flex_row()
                .items_center()
                .gap_1()
                .accessibility_label(tr_opt("等宽字体", "Mono font"))
                .child(ui_radio_pill(
                    "JetBrains Mono",
                    move || !font.system.get(),
                    move || set_system_mono_font(&c_bundled, false),
                ))
                .child(ui_radio_pill(
                    tr("系统", "System"),
                    move || font.system.get(),
                    move || set_system_mono_font(&c_system, true),
                )),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(feature = "desktop", not(target_arch = "wasm32")))]
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

    #[cfg(not(target_arch = "wasm32"))]
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

    #[test]
    fn zh_ui_appends_cjk_mono_fallback() {
        let jb = Some(SharedString::from("JetBrains Mono NL"));
        let bundled_zh = mono_family_for_lang(false, jb.clone(), Lang::Zh).unwrap();
        assert!(
            bundled_zh.starts_with("JetBrains Mono NL, "),
            "{bundled_zh}"
        );
        assert!(bundled_zh.contains(cjk_mono_fallback()), "{bundled_zh}");

        let system_zh = mono_family_for_lang(true, jb, Lang::Zh).unwrap();
        assert!(!system_zh.contains("JetBrains Mono"), "{system_zh}");
        assert!(system_zh.contains("monospace"), "{system_zh}");
    }
}
