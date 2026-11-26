//! General Settings section for choosing curated font sets and UI / monospace font stacks.

#[allow(unused_imports)]
use super::*;

#[derive(Clone, Copy)]
struct FontChoice {
    value: &'static str,
    zh: &'static str,
    en: &'static str,
}

#[derive(Clone, Copy)]
struct FontDraftState {
    font: MonoFont,
    bound: Signal<String>,
    draft: Signal<String>,
    dirty: Signal<bool>,
    edited_rev: Signal<u64>,
    timer: Stored<Option<Spawned<()>>>,
}

const fn choice(value: &'static str, zh: &'static str, en: &'static str) -> FontChoice {
    FontChoice { value, zh, en }
}

const UI_FONT_CHOICES: &[FontChoice] = &[
    choice("", "跟随字体集", "Follow font set"),
    choice(UI_SENTINEL_SYSTEM, "系统默认", "System default"),
    choice("Inter", "Inter", "Inter"),
    choice("PingFang SC", "苹方（PingFang SC）", "PingFang SC"),
    choice("SF Pro Text", "SF Pro Text", "SF Pro Text"),
    choice("Helvetica Neue", "Helvetica Neue", "Helvetica Neue"),
    choice("Microsoft YaHei UI", "微软雅黑 UI", "Microsoft YaHei UI"),
    choice(
        "Noto Sans CJK SC",
        "思源黑体（Noto Sans CJK SC）",
        "Noto Sans CJK SC",
    ),
    choice("Georgia", "Georgia（衬线）", "Georgia (Serif)"),
];

const MONO_FONT_CHOICES: &[FontChoice] = &[
    choice("", "跟随字体集", "Follow font set"),
    choice(
        MONO_SENTINEL_BUNDLED,
        "JetBrains Mono NL（内置）",
        "JetBrains Mono NL (Bundled)",
    ),
    choice(MONO_SENTINEL_SYSTEM, "系统等宽", "System monospace"),
    choice("SF Mono", "SF Mono", "SF Mono"),
    choice("Menlo", "Menlo", "Menlo"),
    choice("Fira Code", "Fira Code", "Fira Code"),
    choice("Cascadia Mono", "Cascadia Mono", "Cascadia Mono"),
    choice(
        "Sarasa Mono SC",
        "更纱黑体等宽（Sarasa Mono SC）",
        "Sarasa Mono SC",
    ),
    choice("Consolas", "Consolas", "Consolas"),
    choice("Monaco", "Monaco", "Monaco"),
];

fn font_preset_label(preset: config::FontPreset) -> (&'static str, &'static str) {
    match preset {
        config::FontPreset::Default => ("默认", "Default"),
        config::FontPreset::System => ("系统原生", "System"),
        config::FontPreset::ModernSans => ("现代无衬线", "Modern Sans"),
        config::FontPreset::Developer => ("开发者", "Developer"),
        config::FontPreset::CjkReading => ("中文阅读", "CJK Reading"),
        config::FontPreset::Classic => ("经典衬线", "Classic"),
    }
}

fn font_preset_summary(preset: config::FontPreset) -> (&'static str, &'static str) {
    match preset {
        config::FontPreset::Default => (
            "系统界面字体 + 内置 JetBrains Mono NL（无连字）",
            "System UI font + bundled JetBrains Mono NL (no ligatures)",
        ),
        config::FontPreset::System => (
            "全部使用操作系统原生界面与等宽字体",
            "Native OS UI and monospaced fonts everywhere",
        ),
        config::FontPreset::ModernSans => (
            "Inter 现代无衬线界面 + 内置 JetBrains Mono NL",
            "Inter modern sans UI + bundled JetBrains Mono NL",
        ),
        config::FontPreset::Developer => (
            "Inter / 系统界面 + Fira Code / JetBrains Mono NL 代码栈",
            "Inter UI + Fira Code / JetBrains Mono NL developer stack",
        ),
        config::FontPreset::CjkReading => (
            "苹方 / 思源黑体中文优先 + 更纱黑体等宽 / JetBrains Mono NL",
            "PingFang / Noto Sans CJK UI + Sarasa Mono / JetBrains Mono NL",
        ),
        config::FontPreset::Classic => (
            "Georgia / 宋体衬线界面 + Menlo / Consolas 经典等宽",
            "Georgia / serif UI + Menlo / Consolas classic monospace",
        ),
    }
}

fn find_choice_title(choices: &[FontChoice], current: &str, lang: Lang) -> &'static str {
    let trimmed = config::sanitize_font_family(current);
    if trimmed.is_empty() {
        return t(lang, choices[0].zh, choices[0].en);
    }
    for choice in choices.iter().skip(1) {
        if choice.value.eq_ignore_ascii_case(&trimmed) {
            return t(lang, choice.zh, choice.en);
        }
    }
    t(lang, "自定义", "Custom")
}

fn matching_choice_value(choices: &[FontChoice], current: &str) -> Option<SharedString> {
    let trimmed = config::sanitize_font_family(current);
    if trimmed.is_empty() {
        return Some(SharedString::from(""));
    }
    choices
        .iter()
        .skip(1)
        .find(|choice| choice.value.eq_ignore_ascii_case(&trimmed))
        .map(|choice| SharedString::from(choice.value))
}

/// Settings -> General font set picker, UI/Mono font selectors, custom stack inputs, and preview.
pub(crate) fn settings_font_section(controls: Arc<Controls>) -> impl IntoElement {
    let font = mono_font();
    let langs = ui_langs();
    let ui_controls = controls.clone();
    let mono_controls = controls.clone();

    view()
        .w_full()
        .px_1()
        .py_0p5()
        .gap_2p5()
        .flex_col()
        .child(font_section_header(font, controls.clone()))
        .child(preset_pills_row(font, controls))
        .child(
            text(move || -> SharedString {
                let (zh, en) = font_preset_summary(font.preset.get());
                t(ui_lang(langs), zh, en).into()
            })
            .text_size(px(TEXT_XS))
            .text_color(fg_muted()),
        )
        .child(font_override_row(
            font,
            ("界面字体", "UI Font"),
            ("选择界面字体", "Select UI font"),
            ("自定义界面字体或字体栈", "Custom UI font or font stack"),
            (
                "字体名或回退栈，如 Inter, PingFang SC",
                "Font or fallback stack, e.g. Inter, sans-serif",
            ),
            UI_FONT_CHOICES,
            font.ui_font,
            move |val| set_ui_font(&ui_controls, val),
        ))
        .child(font_override_row(
            font,
            ("等宽字体", "Mono Font"),
            ("选择等宽字体", "Select monospace font"),
            (
                "自定义等宽字体或字体栈",
                "Custom monospace font or font stack",
            ),
            (
                "字体名或回退栈，如 Fira Code, Menlo",
                "Font or fallback stack, e.g. Fira Code, Menlo",
            ),
            MONO_FONT_CHOICES,
            font.mono_font,
            move |val| set_mono_font(&mono_controls, val),
        ))
        .child(font_live_preview(font))
}

fn font_section_header(font: MonoFont, controls: Arc<Controls>) -> impl IntoElement {
    let is_default_state = move || {
        font.preset.get() == config::FontPreset::Default
            && font.ui_font.get().is_empty()
            && font.mono_font.get().is_empty()
            && !font.system.get()
    };
    let is_customized = move || !font.ui_font.get().is_empty() || !font.mono_font.get().is_empty();

    view()
        .w_full()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(control_text(tr("字体集", "Font Set")).text_color(fg_secondary()))
                .child(when(is_customized, || {
                    view()
                        .px_1p5()
                        .py_0p5()
                        .rounded(px(RADIUS_SM))
                        .bg(accent_blue_soft())
                        .border_1()
                        .border_color(accent_blue_border())
                        .child(
                            text(tr("已自定义", "Customized"))
                                .number_of_lines(1)
                                .text_size(px(TEXT_XS))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(accent_blue_text()),
                        )
                })),
        )
        .child(
            ui_button(ButtonVariant::Secondary, ControlSize::Xs)
                .border_1()
                .border_color(border_control())
                .disabled(is_default_state)
                .accessibility_label(tr_opt("恢复默认字体", "Reset font settings"))
                .on_press(move |_: &()| reset_font_prefs(&controls))
                .child(
                    compact_control_text(tr("恢复默认", "Reset"), TEXT_XS)
                        .text_color(fg_secondary()),
                ),
        )
}

fn preset_pills_row(font: MonoFont, controls: Arc<Controls>) -> impl IntoElement {
    config::FontPreset::ALL.into_iter().fold(
        radio_group()
            .w_full()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_1p5()
            .accessibility_label(tr_opt("选择字体集", "Select font set")),
        |group, preset| {
            let (zh, en) = font_preset_label(preset);
            let click_controls = controls.clone();
            group.child(ui_radio_pill(
                tr(zh, en),
                move || font.preset.get() == preset,
                move || apply_font_preset(&click_controls, preset),
            ))
        },
    )
}

fn font_override_row(
    font: MonoFont,
    title: (&'static str, &'static str),
    select_a11y: (&'static str, &'static str),
    input_a11y: (&'static str, &'static str),
    placeholder_pair: (&'static str, &'static str),
    choices: &'static [FontChoice],
    bound: Signal<String>,
    on_commit: impl Fn(&str) + Clone + 'static,
) -> impl IntoElement {
    let state = FontDraftState {
        font,
        bound,
        draft: signal(bound.get_untracked()),
        dirty: signal(false),
        edited_rev: signal(font.revision.get_untracked()),
        timer: stored(None::<Spawned<()>>),
    };
    effect(move || {
        let external = state.bound.get();
        state.timer.replace(None);
        state.dirty.set(false);
        if state.draft.get_untracked() != external {
            state.draft.set(external);
        }
    });

    view()
        .w_full()
        .gap_1()
        .flex_col()
        .child(
            text(tr(title.0, title.1))
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::MEDIUM)
                .text_color(fg_muted()),
        )
        .child(
            view()
                .w_full()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(font_choice_select(
                    state,
                    select_a11y,
                    choices,
                    on_commit.clone(),
                ))
                .child(font_override_input(
                    state,
                    input_a11y,
                    placeholder_pair,
                    on_commit,
                )),
        )
}

fn font_choice_select(
    state: FontDraftState,
    select_a11y: (&'static str, &'static str),
    choices: &'static [FontChoice],
    on_commit: impl Fn(&str) + 'static,
) -> impl IntoElement {
    let langs = ui_langs();
    let open = signal(false);
    bind_escape_while(
        move || open.get(),
        nsh_core::keymap::When::Always,
        move || open.set(false),
    );
    let items = move || -> Vec<SelectItem> {
        let lang = ui_lang(langs);
        choices
            .iter()
            .map(|c| SelectItem::new(c.value, t(lang, c.zh, c.en)))
            .collect()
    };

    select()
        .items(items)
        .value(move || matching_choice_value(choices, &state.bound.get()))
        .open(move || Some(open.get()))
        .on_open_change(move |event: &SelectEvent| {
            if let SelectEvent::OpenChange(next, _) = event {
                open.set(*next);
            }
        })
        .on_change(move |value: &Option<SharedString>| {
            if let Some(raw) = value.as_ref() {
                state.timer.replace(None);
                state.dirty.set(false);
                state.draft.set(raw.to_string());
                on_commit(raw.as_ref());
            }
        })
        .cover(false)
        .placement(Placement::Bottom)
        .align(Align::Start)
        .offset(px(4.0))
        .margin(px(8.0))
        .accessibility_label(tr_opt(select_a11y.0, select_a11y.1))
        .popup_style(|s| ui_popup_style(s).w(px(236.0)))
        .popup_starting_style(ui_popup_starting_style)
        .popup_ending_style(ui_popup_ending_style)
        .list_style(|s| s.max_h(px(260.0)).p_1p5())
        .item_class("focus:bg-accent hover:bg-accent")
        .item_style(|s| {
            s.w_full()
                .h(px(26.0))
                .px_2()
                .rounded(px(RADIUS_MD))
                .flex_row()
                .items_center()
                .selected(|sel| sel.bg(accent_blue_soft()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
        })
        .render_item(move |entry: &SelectEntry| {
            let val = entry.value.clone();
            let active =
                move || matching_choice_value(choices, &state.bound.get()).as_ref() == Some(&val);
            view()
                .w_full()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    text(entry.title.clone())
                        .number_of_lines(1)
                        .text_size(px(TEXT_SM))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || {
                            if active() {
                                accent_blue_text()
                            } else {
                                fg_primary()
                            }
                        }),
                )
        })
        .trigger(
            view()
                .w(px(176.0))
                .h(px(CONTROL_H))
                .px_2p5()
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_1p5()
                .rounded(px(RADIUS_MD))
                .border_1()
                .border_color(border_control())
                .bg(bg_control())
                .hover(|s| s.bg(bg_control_hover()))
                .active(|s| s.bg(bg_control_active()))
                .transition_colors()
                .duration(MOTION_INSTANT_MS)
                .ease_out()
                .focus_visible(apply_focus_ring)
                .child(
                    text(move || -> SharedString {
                        find_choice_title(choices, &state.bound.get(), ui_lang(langs)).into()
                    })
                    .flex_1()
                    .min_w(px(0.0))
                    .number_of_lines(1)
                    .text_overflow(EllipsizeMode::Tail)
                    .text_size(px(TEXT_SM))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg_primary()),
                )
                .child(glyph_px(10.0, || Glyph::Chevron, || color(fg_muted()))),
        )
}

fn font_override_input(
    state: FontDraftState,
    input_a11y: (&'static str, &'static str),
    placeholder_pair: (&'static str, &'static str),
    on_commit: impl Fn(&str) + Clone + 'static,
) -> impl IntoElement {
    let commit_debounce = on_commit.clone();
    let commit_submit = on_commit.clone();
    let commit_blur = on_commit;

    ui_input_a11y(input(), tr(input_a11y.0, input_a11y.1))
        .value(state.draft)
        .placeholder(tr(placeholder_pair.0, placeholder_pair.1))
        .placeholder_color(move || color(fg_muted()))
        .flex_1()
        .min_w(px(140.0))
        .h(px(CONTROL_H))
        .px_2p5()
        .rounded(px(RADIUS_MD))
        .bg(bg_code())
        .border_1()
        .border_color(border_control())
        .focus(|s| s.border_color(accent_blue()))
        .text_size(px(TEXT_SM))
        .text_color(fg_primary())
        .on_change(move |text_val: &str| {
            let owned = text_val.to_string();
            let rev = state.font.revision.get_untracked();
            state.edited_rev.set(rev);
            state.dirty.set(true);
            state.draft.set(owned.clone());
            let commit_task = commit_debounce.clone();
            state
                .timer
                .replace(Some(set_timeout(Duration::from_millis(350), move || {
                    if state.font.revision.get_untracked() == rev && state.dirty.get_untracked() {
                        state.dirty.set(false);
                        commit_task(&owned);
                    }
                })));
        })
        .on_submit(move |text_val: &str| {
            state.timer.replace(None);
            state.dirty.set(false);
            commit_submit(text_val);
            let synced = state.bound.get_untracked();
            if state.draft.get_untracked() != synced {
                state.draft.set(synced);
            }
        })
        .on_blur(move || {
            state.timer.replace(None);
            if state.dirty.get_untracked()
                && state.edited_rev.get_untracked() == state.font.revision.get_untracked()
            {
                state.dirty.set(false);
                let current = state.draft.get_untracked();
                commit_blur(&current);
                let synced = state.bound.get_untracked();
                if state.draft.get_untracked() != synced {
                    state.draft.set(synced);
                }
            }
        })
}

fn font_live_preview(font: MonoFont) -> impl IntoElement {
    let langs = ui_langs();
    view()
        .w_full()
        .p_2p5()
        .gap_1()
        .flex_col()
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(border_subtle())
        .bg(bg_code())
        .child(
            text(tr("实时预览", "Live Preview"))
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg_muted()),
        )
        .child(
            text(tr(
                "界面示例：zniff 抓包面板 · 中文与 English 排版 0123456789",
                "UI sample: zniff Inspector · Clean typography & 中文 0123456789",
            ))
            .text_size(px(TEXT_SM))
            .text_color(fg_primary()),
        )
        .child(
            mono("GET https://api.vicanso.dev/v1/items?limit=20 -> 200 OK · 42 ms != ===")
                .text_size(px(TEXT_XS))
                .text_color(accent_blue_text()),
        )
        .child(
            text(move || -> SharedString {
                let lang = ui_lang(langs);
                let sys_label = t(lang, "系统默认", "System");
                let ui_resolved = resolve_ui_family(font.preset.get(), &font.ui_font.get(), lang);
                let mono_resolved = resolve_mono_family(
                    font.preset.get(),
                    font.system.get(),
                    &font.mono_font.get(),
                    font.bundled.get(),
                    lang,
                );
                let ui_name = ui_resolved.as_deref().unwrap_or(sys_label);
                let mono_name = mono_resolved.as_deref().unwrap_or(sys_label);
                match lang {
                    Lang::Zh => format!("生效栈：界面 [{ui_name}] · 等宽 [{mono_name}]").into(),
                    Lang::En => format!("Active: UI [{ui_name}] · Mono [{mono_name}]").into(),
                }
            })
            .text_size(px(TEXT_XS))
            .text_color(fg_muted()),
        )
}
