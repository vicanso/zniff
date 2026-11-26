//! UI and monospaced font sets, custom font overrides, and bundled JetBrains Mono NL loading.
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

pub(crate) const UI_SENTINEL_SYSTEM: &str = "system";
pub(crate) const MONO_SENTINEL_BUNDLED: &str = "bundled";
pub(crate) const MONO_SENTINEL_SYSTEM: &str = "system";

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

/// User font preferences (preset font set, custom overrides, legacy system mono flag)
/// plus the bundled JetBrains Mono NL family once registered.
#[derive(Clone, Copy)]
pub(crate) struct MonoFont {
    pub(crate) preset: Signal<config::FontPreset>,
    pub(crate) ui_font: Signal<String>,
    pub(crate) mono_font: Signal<String>,
    /// True draws mono text in the system's monospaced font when preset is `Default`.
    pub(crate) system: Signal<bool>,
    /// Bumped whenever a preset/reset transition supersedes pending input drafts.
    pub(crate) revision: Signal<u64>,
    /// The bundled family name, `None` until registered (or when it failed).
    pub(crate) bundled: Signal<Option<SharedString>>,
}

fn spawn_bundled_font_loader(font: MonoFont) {
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
            let needs_bundled =
                uses_bundled_mono(font.preset.get(), font.system.get(), &font.mono_font.get());
            if !needs_bundled || font.bundled.get().is_some() || loading_eff.get() {
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
}

/// Registers the bundled faces and keeps theme `font-sans` and `font-mono`
/// synchronized with user preferences. Called once from `App`.
pub(crate) fn provide_font_prefs(
    preset: config::FontPreset,
    ui_font: String,
    mono_font: String,
    system: bool,
) -> MonoFont {
    let ui_font = config::sanitize_font_family(&ui_font);
    let mono_font = config::sanitize_font_family(&mono_font);
    let effective_system = if mono_font.eq_ignore_ascii_case(MONO_SENTINEL_SYSTEM) {
        true
    } else if mono_font.eq_ignore_ascii_case(MONO_SENTINEL_BUNDLED) {
        false
    } else if mono_font.is_empty() {
        preset == config::FontPreset::System || (preset == config::FontPreset::Default && system)
    } else {
        false
    };
    let font = MonoFont {
        preset: signal(preset),
        ui_font: signal(ui_font),
        mono_font: signal(mono_font),
        system: signal(effective_system),
        revision: signal(0),
        bundled: signal(None),
    };
    provide_context(font);
    spawn_bundled_font_loader(font);
    let langs = ui_langs();
    effect(move || {
        let lang = ui_lang(langs);
        let preset = font.preset.get();
        let ui_override = font.ui_font.get();
        let mono_override = font.mono_font.get();
        let ui_family = resolve_ui_family(preset, &ui_override, lang);
        let mono_family = resolve_mono_family(
            preset,
            font.system.get(),
            &mono_override,
            font.bundled.get(),
            lang,
        );
        set_tokens(
            build_nsh_theme_with_fonts(ui_family.as_deref(), mono_family.as_deref()).into_tokens(),
        );
    });
    font
}

/// Convenience initializer keeping older test harnesses concise.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn provide_mono_font(system: bool) -> MonoFont {
    let preset = if system {
        config::FontPreset::System
    } else {
        config::FontPreset::Default
    };
    provide_font_prefs(preset, String::new(), String::new(), system)
}

/// Returns whether the current configuration benefits from loading the bundled JetBrains Mono NL faces.
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
pub(crate) fn uses_bundled_mono(
    preset: config::FontPreset,
    system: bool,
    mono_override: &str,
) -> bool {
    let cleaned = config::sanitize_font_family(mono_override);
    if !cleaned.is_empty() {
        if cleaned.eq_ignore_ascii_case(MONO_SENTINEL_BUNDLED) {
            return true;
        }
        if cleaned.eq_ignore_ascii_case(MONO_SENTINEL_SYSTEM) {
            return false;
        }
        return cleaned.to_ascii_lowercase().contains("jetbrains mono");
    }
    match preset {
        config::FontPreset::Default => !system,
        config::FontPreset::ModernSans
        | config::FontPreset::Developer
        | config::FontPreset::CjkReading => true,
        config::FontPreset::System | config::FontPreset::Classic => false,
    }
}

/// Platform-appropriate UI sans fallback list used when the UI is in Chinese.
pub(crate) fn cjk_ui_fallback() -> &'static str {
    #[cfg(target_arch = "wasm32")]
    {
        "PingFang SC, Microsoft YaHei UI, Noto Sans CJK SC, system-ui, sans-serif"
    }
    #[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
    {
        "PingFang SC, Hiragino Sans GB, system-ui, sans-serif"
    }
    #[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
    {
        "Microsoft YaHei UI, Microsoft YaHei, Segoe UI, system-ui, sans-serif"
    }
    #[cfg(all(
        not(target_arch = "wasm32"),
        not(target_os = "macos"),
        not(target_os = "windows")
    ))]
    {
        "Noto Sans CJK SC, WenQuanYi Micro Hei, system-ui, sans-serif"
    }
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

fn is_generic_font_keyword(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "sans-serif"
            | "serif"
            | "monospace"
            | "system-ui"
            | "ui-sans-serif"
            | "ui-serif"
            | "ui-monospace"
            | "cursive"
            | "fantasy"
    )
}

fn replace_bundled_token(stack: &str, bundled: Option<&str>) -> String {
    let replacement = bundled.filter(|s| !s.is_empty());
    let parts: Vec<String> = stack
        .split(',')
        .filter_map(|raw| {
            let item = raw.trim();
            if item.is_empty() {
                return None;
            }
            let unquoted = item.trim_matches(['"', '\'']).trim();
            if unquoted.eq_ignore_ascii_case("jetbrains mono nl") {
                Some(replacement.unwrap_or("JetBrains Mono NL").to_string())
            } else {
                Some(item.to_string())
            }
        })
        .collect();
    parts.join(", ")
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

/// Resolves the active `font-sans` value from the chosen font set and UI override.
pub(crate) fn resolve_ui_family(
    preset: config::FontPreset,
    ui_override: &str,
    lang: Lang,
) -> Option<SharedString> {
    let cleaned = config::sanitize_font_family(ui_override);
    if !cleaned.is_empty() {
        if cleaned.eq_ignore_ascii_case(UI_SENTINEL_SYSTEM) {
            return None;
        }
        if cleaned.contains(',') || is_generic_font_keyword(&cleaned) {
            return Some(SharedString::from(cleaned));
        }
        let stack = match lang {
            Lang::Zh => format!("{cleaned}, {}", cjk_ui_fallback()),
            Lang::En => {
                format!(
                    "{cleaned}, system-ui, -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif"
                )
            }
        };
        return Some(SharedString::from(stack));
    }

    let preset_stack = match preset {
        config::FontPreset::Default | config::FontPreset::System => return None,
        config::FontPreset::ModernSans => {
            "Inter, -apple-system, BlinkMacSystemFont, Segoe UI, PingFang SC, Microsoft YaHei UI, Noto Sans CJK SC, sans-serif"
        }
        config::FontPreset::Developer => {
            "Inter, SF Pro Text, Segoe UI, PingFang SC, Noto Sans CJK SC, sans-serif"
        }
        config::FontPreset::CjkReading => {
            "PingFang SC, Hiragino Sans GB, Microsoft YaHei UI, Noto Sans CJK SC, system-ui, sans-serif"
        }
        config::FontPreset::Classic => "Georgia, Songti SC, STSong, Noto Serif CJK SC, serif",
    };
    Some(SharedString::from(preset_stack))
}

/// Resolves the active `font-mono` value from font set, legacy flag, override, and bundled status.
pub(crate) fn resolve_mono_family(
    preset: config::FontPreset,
    system: bool,
    mono_override: &str,
    bundled: Option<SharedString>,
    lang: Lang,
) -> Option<SharedString> {
    let cleaned = config::sanitize_font_family(mono_override);
    let bundled_ref = bundled.as_deref();
    if !cleaned.is_empty() {
        if cleaned.eq_ignore_ascii_case(MONO_SENTINEL_BUNDLED) {
            return mono_family_for_lang(false, bundled, lang);
        }
        if cleaned.eq_ignore_ascii_case(MONO_SENTINEL_SYSTEM) {
            return mono_family_for_lang(true, None, lang);
        }
        if cleaned.contains(',') {
            return Some(SharedString::from(replace_bundled_token(
                &cleaned,
                bundled_ref,
            )));
        }
        if is_generic_font_keyword(&cleaned) {
            return Some(SharedString::from(cleaned));
        }
        if cleaned.eq_ignore_ascii_case("jetbrains mono")
            || cleaned.eq_ignore_ascii_case("jetbrains mono nl")
        {
            return mono_family_for_lang(false, bundled, lang);
        }
        let stack = match lang {
            Lang::Zh => format!("{cleaned}, {}", cjk_mono_fallback()),
            Lang::En => format!("{cleaned}, monospace"),
        };
        return Some(SharedString::from(stack));
    }

    match preset {
        config::FontPreset::Default => mono_family_for_lang(system, bundled, lang),
        config::FontPreset::System => mono_family_for_lang(true, None, lang),
        config::FontPreset::ModernSans => mono_family_for_lang(false, bundled, lang),
        config::FontPreset::Developer => {
            let base = "Fira Code, JetBrains Mono NL, SF Mono, Cascadia Mono, Menlo, Consolas";
            let stack = match lang {
                Lang::Zh => format!("{base}, {}", cjk_mono_fallback()),
                Lang::En => format!("{base}, monospace"),
            };
            Some(SharedString::from(replace_bundled_token(
                &stack,
                bundled_ref,
            )))
        }
        config::FontPreset::CjkReading => {
            let stack = "Sarasa Mono SC, JetBrains Mono NL, PingFang SC, Noto Sans Mono CJK SC, Menlo, Consolas, monospace";
            Some(SharedString::from(replace_bundled_token(
                stack,
                bundled_ref,
            )))
        }
        config::FontPreset::Classic => {
            let stack = match lang {
                Lang::Zh => format!(
                    "Menlo, Monaco, Consolas, DejaVu Sans Mono, {}",
                    cjk_mono_fallback()
                ),
                Lang::En => "Menlo, Monaco, Consolas, DejaVu Sans Mono, monospace".to_string(),
            };
            Some(SharedString::from(stack))
        }
    }
}

/// The app's [`MonoFont`].
pub(crate) fn mono_font() -> MonoFont {
    use_context().expect("font preferences context")
}

/// Applies a preset font set and clears custom font overrides so the preset takes full effect.
pub(crate) fn apply_font_preset(controls: &Controls, preset: config::FontPreset) {
    let font = mono_font();
    // Advance revision before dismissing keyboard so a late blur from an active
    // custom-font field cannot overwrite the newly chosen preset.
    font.revision.update(|n| *n += 1);
    dismiss_keyboard();
    let sys = preset == config::FontPreset::System;
    font.preset.set(preset);
    font.ui_font.set(String::new());
    font.mono_font.set(String::new());
    font.system.set(sys);
    save_config_or_notify(controls, PrefName::Font, move |cfg| {
        cfg.font_preset = preset;
        cfg.ui_font.clear();
        cfg.mono_font.clear();
        cfg.system_mono_font = sys;
    });
}

/// Sets or clears the UI font override (`""` follows the selected font set).
pub(crate) fn set_ui_font(controls: &Controls, raw: &str) {
    let cleaned = config::sanitize_font_family(raw);
    let font = mono_font();
    if font.ui_font.get_untracked() == cleaned {
        return;
    }
    font.ui_font.set(cleaned.clone());
    save_config_or_notify(controls, PrefName::Font, move |cfg| {
        cfg.ui_font = cleaned;
    });
}

/// Sets or clears the monospace font override (`""` follows the selected font set).
pub(crate) fn set_mono_font(controls: &Controls, raw: &str) {
    let cleaned = config::sanitize_font_family(raw);
    let font = mono_font();
    if font.mono_font.get_untracked() == cleaned {
        return;
    }
    font.mono_font.set(cleaned.clone());
    let next_sys = if cleaned.eq_ignore_ascii_case(MONO_SENTINEL_SYSTEM) {
        true
    } else if cleaned.eq_ignore_ascii_case(MONO_SENTINEL_BUNDLED) {
        false
    } else if cleaned.is_empty() {
        font.preset.get_untracked() == config::FontPreset::System
    } else {
        false
    };
    font.system.set(next_sys);
    save_config_or_notify(controls, PrefName::Font, move |cfg| {
        cfg.mono_font = cleaned;
        cfg.system_mono_font = next_sys;
    });
}

/// Resets font set and overrides back to `FontPreset::Default`.
pub(crate) fn reset_font_prefs(controls: &Controls) {
    apply_font_preset(controls, config::FontPreset::Default);
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

    #[test]
    fn resolves_all_presets_and_custom_overrides_by_lang() {
        let jb = Some(SharedString::from("JetBrains Mono NL"));
        for preset in config::FontPreset::ALL {
            let ui_en = resolve_ui_family(preset, "", Lang::En);
            let mono_en = resolve_mono_family(preset, false, "", jb.clone(), Lang::En);
            let mono_zh = resolve_mono_family(preset, false, "", jb.clone(), Lang::Zh);
            match preset {
                config::FontPreset::Default => {
                    assert_eq!(ui_en, None);
                    assert_eq!(mono_en, jb);
                    assert!(mono_zh.unwrap().contains(cjk_mono_fallback()));
                }
                config::FontPreset::System => {
                    assert_eq!(ui_en, None);
                    assert_eq!(mono_en, None);
                    assert!(mono_zh.unwrap().contains("monospace"));
                }
                config::FontPreset::ModernSans => {
                    assert!(ui_en.unwrap().starts_with("Inter, "));
                    assert_eq!(mono_en, jb);
                }
                config::FontPreset::Developer => {
                    assert!(ui_en.unwrap().starts_with("Inter, "));
                    assert!(mono_en.unwrap().starts_with("Fira Code, JetBrains Mono NL"));
                    assert!(mono_zh.unwrap().contains(cjk_mono_fallback()));
                }
                config::FontPreset::CjkReading => {
                    assert!(ui_en.unwrap().starts_with("PingFang SC, "));
                    assert!(mono_en.unwrap().starts_with("Sarasa Mono SC, "));
                }
                config::FontPreset::Classic => {
                    assert!(ui_en.unwrap().starts_with("Georgia, "));
                    assert!(mono_en.unwrap().starts_with("Menlo, "));
                }
            }
        }

        let single_ui = resolve_ui_family(config::FontPreset::Classic, "Inter", Lang::Zh).unwrap();
        assert!(single_ui.starts_with("Inter, "), "{single_ui}");
        assert!(single_ui.contains(cjk_ui_fallback()), "{single_ui}");

        let stack_ui = resolve_ui_family(
            config::FontPreset::Default,
            "My Sans, PingFang SC, sans-serif",
            Lang::En,
        )
        .unwrap();
        assert_eq!(stack_ui.as_ref(), "My Sans, PingFang SC, sans-serif");

        assert_eq!(
            resolve_ui_family(config::FontPreset::ModernSans, UI_SENTINEL_SYSTEM, Lang::En),
            None
        );
        assert_eq!(
            resolve_mono_family(
                config::FontPreset::Classic,
                false,
                MONO_SENTINEL_BUNDLED,
                jb.clone(),
                Lang::En
            ),
            jb
        );
        assert_eq!(
            resolve_mono_family(
                config::FontPreset::Developer,
                false,
                MONO_SENTINEL_SYSTEM,
                jb,
                Lang::En
            ),
            None
        );

        assert!(uses_bundled_mono(config::FontPreset::Default, false, ""));
        assert!(!uses_bundled_mono(config::FontPreset::Default, true, ""));
        assert!(!uses_bundled_mono(config::FontPreset::System, false, ""));
        assert!(uses_bundled_mono(config::FontPreset::Developer, false, ""));
        assert!(!uses_bundled_mono(
            config::FontPreset::Developer,
            false,
            "Fira Code"
        ));
        assert!(uses_bundled_mono(
            config::FontPreset::Classic,
            false,
            MONO_SENTINEL_BUNDLED
        ));
    }
}
