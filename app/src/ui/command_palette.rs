#[allow(unused_imports)]
use super::*;

use nsh_base::{CommandEntry, CommandGroup, CommandItem};

pub(crate) fn build_command_items(
    lang: Lang,
    running: bool,
    proxy_on: bool,
    has_rows: bool,
    has_selection: bool,
    layout_mode: config::LayoutMode,
    sidebar_collapsed: bool,
    pretty: bool,
    default_json_tree: bool,
    adb_count: usize,
) -> Vec<CommandItem> {
    let g_capture = t(lang, "抓包", "Capture");
    let g_export = t(lang, "导出与分析", "Export & Tools");
    let g_view = t(lang, "视图与设置", "View & Settings");

    let mut items = vec![
        CommandItem::new(
            "toggle_capture",
            if running {
                t(lang, "停止抓包", "Stop capture")
            } else {
                t(lang, "开始抓包", "Start capture")
            },
        )
        .group(g_capture)
        .keywords(["capture", "start", "stop", "proxy", "抓包", "开始", "停止"]),
        CommandItem::new("new_request", t(lang, "构造请求", "New request"))
            .group(g_capture)
            .keywords(["compose", "request", "curl", "http", "构造", "请求"]),
        CommandItem::new(
            "replay_selected",
            t(lang, "重放当前选中流量", "Replay selected flow"),
        )
        .group(g_capture)
        .shortcut("mod+r")
        .disabled(!has_selection)
        .keywords(["replay", "resend", "repeat", "重放", "重发"]),
        CommandItem::new(
            "focus_filter",
            t(lang, "聚焦主过滤框", "Focus filter input"),
        )
        .group(g_capture)
        .shortcut("mod+f")
        .keywords(["filter", "search", "find", "过滤", "搜索"]),
        CommandItem::new("clear_all", t(lang, "清空全部流量", "Clear all flows"))
            .group(g_capture)
            .shortcut("mod+k")
            .keywords(["clear", "reset", "trash", "清空"]),
        CommandItem::new(
            "clear_unpinned",
            t(lang, "清空未星标流量", "Clear unstarred flows"),
        )
        .group(g_capture)
        .keywords(["clear", "unstarred", "pin", "星标", "清空"]),
    ];

    #[cfg(not(target_arch = "wasm32"))]
    {
        let g_launch = t(lang, "免配抓包", "Launch");
        items.insert(
            1,
            CommandItem::new(
                "toggle_sysproxy",
                if proxy_on {
                    t(lang, "关闭系统代理", "Disable system proxy")
                } else {
                    t(lang, "开启系统代理", "Enable system proxy")
                },
            )
            .group(g_capture)
            .checked(proxy_on)
            .keywords(["system", "proxy", "sysproxy", "系统代理"]),
        );

        items.push(
            CommandItem::new(
                "launch_browser",
                t(
                    lang,
                    "启动免证书隔离浏览器 (Chromium)",
                    "Launch isolated browser",
                ),
            )
            .group(g_launch)
            .keywords(["browser", "chrome", "chromium", "spki", "浏览器"]),
        );
        items.push(
            CommandItem::new(
                "launch_terminal",
                t(lang, "启动预注入代理终端", "Launch proxied terminal"),
            )
            .group(g_launch)
            .keywords(["terminal", "shell", "term", "env", "终端"]),
        );
        items.push(
            CommandItem::new(
                "toggle_adb",
                if adb_count > 0 {
                    format!(
                        "{} (ADB · {adb_count})",
                        t(
                            lang,
                            "断开 Android ADB 代理",
                            "Disconnect Android ADB proxy"
                        )
                    )
                } else {
                    t(lang, "配置 Android ADB 代理", "Configure Android ADB proxy").to_string()
                },
            )
            .group(g_launch)
            .checked(adb_count > 0)
            .keywords(["adb", "android", "usb", "mobile", "安卓"]),
        );
        items.push(
            CommandItem::new("copy_env", t(lang, "复制代理环境变量", "Copy proxy env"))
                .group(g_launch)
                .keywords(["env", "export", "proxy", "环境变量"]),
        );
        items.push(
            CommandItem::new("copy_mcp", t(lang, "复制 MCP 配置 JSON", "Copy MCP config"))
                .group(g_launch)
                .keywords(["mcp", "ai", "claude", "cursor", "配置"]),
        );
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (proxy_on, adb_count);

    items.push(
        CommandItem::new("export_har", t(lang, "导出 HAR 1.2", "Export HAR 1.2"))
            .group(g_export)
            .keywords(["har", "export", "导出"]),
    );
    items.push(
        CommandItem::new(
            "export_diagnostics",
            t(lang, "导出诊断包…", "Export Diagnostics…"),
        )
        .group(g_export)
        .keywords([
            "diagnostics",
            "debug",
            "issue",
            "bug",
            "support",
            "export",
            "诊断",
            "排查",
            "导出",
        ]),
    );
    items.push(
        CommandItem::new(
            "export_html",
            t(lang, "导出 HTML 快照", "Export HTML snapshot"),
        )
        .group(g_export)
        .keywords(["html", "snapshot", "export", "快照", "导出"]),
    );
    items.push(
        CommandItem::new(
            "export_openapi_json",
            t(lang, "导出 OpenAPI 3.0 JSON", "Export OpenAPI 3.0 JSON"),
        )
        .group(g_export)
        .keywords(["openapi", "swagger", "json", "api", "导出"]),
    );
    items.push(
        CommandItem::new(
            "export_openapi_yaml",
            t(lang, "导出 OpenAPI 3.0 YAML", "Export OpenAPI 3.0 YAML"),
        )
        .group(g_export)
        .keywords(["openapi", "swagger", "yaml", "yml", "api", "导出"]),
    );
    #[cfg(not(target_arch = "wasm32"))]
    {
        items.push(
            CommandItem::new(
                "copy_openapi_yaml",
                t(lang, "复制 OpenAPI 3.0 YAML", "Copy OpenAPI 3.0 YAML"),
            )
            .group(g_export)
            .keywords(["openapi", "swagger", "yaml", "copy", "复制"]),
        );
        items.push(
            CommandItem::new(
                "export_mock_dir",
                t(lang, "生成本地 Mock 目录…", "Export local Mock directory…"),
            )
            .group(g_export)
            .keywords(["mock", "map-local", "stub", "本地", "目录"]),
        );
        items.push(
            CommandItem::new(
                "import_har",
                t(lang, "导入 HAR / Charles 会话…", "Import HAR / .chlsj…"),
            )
            .group(g_export)
            .keywords(["import", "har", "chlsj", "charles", "导入"]),
        );
        items.push(
            CommandItem::new(
                "import_rules",
                t(
                    lang,
                    "导入 Whistle / Charles 规则…",
                    "Import Whistle / Charles rules…",
                ),
            )
            .group(g_export)
            .keywords(["import", "whistle", "charles", "xml", "txt", "迁移", "导入"]),
        );
    }
    items.push(
        CommandItem::new(
            "sequence_diagram",
            t(
                lang,
                "生成时序图 / 瀑布图 / 依赖图",
                "Sequence / waterfall / dependency diagram",
            ),
        )
        .group(g_export)
        .disabled(!has_rows)
        .keywords([
            "sequence",
            "mermaid",
            "waterfall",
            "graph",
            "时序图",
            "瀑布图",
        ]),
    );

    items.push(
        CommandItem::new(
            "toggle_layout",
            if layout_mode == config::LayoutMode::Split {
                t(lang, "切换为上下分栏布局", "Switch to Top/Bottom layout")
            } else {
                t(lang, "切换为左右分栏布局", "Switch to Split layout")
            },
        )
        .group(g_view)
        .keywords(["layout", "split", "topbottom", "布局", "分栏"]),
    );
    items.push(
        CommandItem::new(
            "toggle_sidebar",
            if sidebar_collapsed {
                t(lang, "展开域名/应用分面侧栏", "Show domains & apps sidebar")
            } else {
                t(lang, "收起域名/应用分面侧栏", "Hide domains & apps sidebar")
            },
        )
        .group(g_view)
        .keywords(["sidebar", "facets", "domains", "侧栏", "域名"]),
    );
    items.push(
        CommandItem::new(
            "toggle_pretty",
            if pretty {
                t(lang, "切换为原文展示", "Switch to Raw body view")
            } else {
                t(lang, "切换为默认美化展示", "Switch to Pretty body view")
            },
        )
        .group(g_view)
        .checked(pretty)
        .keywords(["pretty", "raw", "format", "美化", "原文"]),
    );
    items.push(
        CommandItem::new(
            "toggle_json_tree",
            if default_json_tree {
                t(
                    lang,
                    "关闭默认 JSON 树形视图",
                    "Disable JSON tree by default",
                )
            } else {
                t(
                    lang,
                    "开启默认 JSON 树形视图",
                    "Enable JSON tree by default",
                )
            },
        )
        .group(g_view)
        .checked(default_json_tree)
        .keywords(["json", "tree", "树形"]),
    );
    items.push(
        CommandItem::new(
            "show_summary",
            t(lang, "回到当前列表统计视图", "Show filtered summary"),
        )
        .group(g_view)
        .shortcut("escape")
        .disabled(!has_selection)
        .keywords(["summary", "stats", "统计"]),
    );
    items.push(
        CommandItem::new(
            "theme_system",
            t(lang, "主题：跟随系统", "Theme: Follow System"),
        )
        .group(g_view)
        .keywords(["theme", "system", "appearance", "主题", "系统"]),
    );
    items.push(
        CommandItem::new("theme_light", t(lang, "主题：浅色模式", "Theme: Light"))
            .group(g_view)
            .keywords(["theme", "light", "appearance", "主题", "浅色"]),
    );
    items.push(
        CommandItem::new("theme_dark", t(lang, "主题：深色模式", "Theme: Dark"))
            .group(g_view)
            .keywords(["theme", "dark", "appearance", "主题", "深色"]),
    );
    items.push(
        CommandItem::new("zoom_in", t(lang, "放大界面与字体", "Zoom in UI & font"))
            .group(g_view)
            .shortcut("mod+=")
            .keywords(["zoom", "font", "scale", "larger", "放大", "缩放", "字体"]),
    );
    items.push(
        CommandItem::new("zoom_out", t(lang, "缩小界面与字体", "Zoom out UI & font"))
            .group(g_view)
            .shortcut("mod+-")
            .keywords(["zoom", "font", "scale", "smaller", "缩小", "缩放", "字体"]),
    );
    items.push(
        CommandItem::new(
            "zoom_reset",
            t(lang, "重置界面缩放 (100%)", "Reset UI zoom (100%)"),
        )
        .group(g_view)
        .shortcut("mod+0")
        .keywords(["zoom", "reset", "100%", "重置", "缩放"]),
    );
    items.push(
        CommandItem::new("open_settings", t(lang, "打开设置", "Open Settings"))
            .group(g_view)
            .shortcut("mod+,")
            .keywords([
                "settings",
                "preferences",
                "config",
                "设置",
                "偏好",
                "证书",
                "上游",
                "安全",
                "中继",
            ]),
    );
    items.push(
        CommandItem::new(
            "open_shortcuts",
            t(lang, "快捷键速查", "Keyboard shortcuts"),
        )
        .group(g_view)
        .shortcut("mod+/")
        .keywords(["shortcut", "key", "keymap", "help", "快捷键", "帮助"]),
    );
    items.push(
        CommandItem::new("open_about", t(lang, "关于 zniff", "About zniff"))
            .group(g_view)
            .keywords(["about", "version", "关于", "版本"]),
    );

    items
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn command_palette_modal(
    open: Signal<bool>,
    shortcuts_open: Signal<bool>,
    filter_focus: ElementRef,
    controls: Arc<Controls>,
    running: Signal<bool>,
    proxy_on: Signal<bool>,
    rows: Signal<Vec<Arc<RowLine>>>,
    filter: Signal<String>,
    pretty: Signal<bool>,
    profiles: ProfileSignals,
    compose: Compose,
    seq_builder: SeqBuilder,
    selected: Signal<Option<u64>>,
    baseline: Signal<Option<u64>>,
    shown: Shown,
) -> impl IntoElement {
    when(
        move || open.get(),
        move || {
            let langs = ui_langs();
            let exec_controls = controls.clone();
            let backdrop = view()
                .absolute()
                .inset_0()
                .bg(AdaptiveColor::new(rgba(0x0F172A5C), rgba(0x0000008F)))
                .on_press(move || open.set(false));

            let palette = nsh_base::command()
                .auto_focus(true)
                .items(move || {
                    let lang = ui_lang(langs);
                    #[cfg(not(target_arch = "wasm32"))]
                    let adb_count = launcher::configured_adb_count();
                    #[cfg(target_arch = "wasm32")]
                    let adb_count = 0usize;
                    build_command_items(
                        lang,
                        running.get(),
                        proxy_on.get(),
                        !rows.with(Vec::is_empty),
                        selected.get().is_some(),
                        profiles.layout_mode.get(),
                        profiles.sidebar_collapsed.get(),
                        pretty.get(),
                        profiles.default_json_tree.get(),
                        adb_count,
                    )
                })
                .placeholder(move || {
                    Some(SharedString::from(tr(
                        "输入命令或关键词… (↑↓ 选择，Enter 执行，Esc 关闭)",
                        "Type a command or keyword… (↑↓ navigate, Enter run, Esc close)",
                    )()))
                })
                .search_bar_class(
                    "flex flex-row items-center gap-2 px-3 py-2.5 border-b border-border",
                )
                .search_class("flex-1 text-sm bg-transparent")
                .list_style(|s| s.max_h(px(360.0)))
                .item_class(
                    "w-full px-3 py-2 rounded-md cursor-pointer focus:bg-accent hover:bg-accent",
                )
                .group_class("px-3 pt-2 pb-1 text-xs font-semibold text-muted-foreground")
                .separator_class("my-1 h-px bg-border")
                .render_group(|group: &CommandGroup| {
                    text(group.heading.clone())
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg_muted())
                        .into_element()
                })
                .render_item(|entry: &CommandEntry| {
                    let label_color = if entry.disabled {
                        fg_muted()
                    } else {
                        fg_primary()
                    };
                    let row = view()
                        .w_full()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            text(entry.label.clone())
                                .flex_1()
                                .number_of_lines(1)
                                .text_size(px(12.5))
                                .text_color(label_color),
                        );
                    if let Some(shortcut) = entry.shortcut_label.clone() {
                        row.child(
                            mono(shortcut)
                                .text_size(px(11.0))
                                .px_1p5()
                                .py_0p5()
                                .rounded(px(4.0))
                                .border_1()
                                .border_color(border_control())
                                .bg(bg_control())
                                .text_color(fg_muted()),
                        )
                        .into_element()
                    } else {
                        row.into_element()
                    }
                })
                .empty(
                    view()
                        .w_full()
                        .py_6()
                        .items_center()
                        .justify_center()
                        .child(
                            text(tr("没有匹配的命令", "No matching commands"))
                                .text_size(px(12.0))
                                .text_color(fg_muted()),
                        ),
                )
                .on_cancel(move || {
                    open.set(false);
                })
                .on_select(move |id: &SharedString| {
                    open.set(false);
                    let controls = exec_controls.clone();
                    match id.as_str() {
                        "toggle_capture" => toggle_capture(controls),
                        #[cfg(not(target_arch = "wasm32"))]
                        "toggle_sysproxy" => {
                            toggle_system_proxy(controls);
                        }
                        "new_request" => begin_new_request(compose, selected),
                        "replay_selected" => {
                            replay_flow(&controls, selected.get(), ReplayOverride::default());
                        }
                        "focus_filter" => filter_focus.focus(),
                        "clear_all" => clear_captured(
                            &controls,
                            selected,
                            baseline,
                            compose,
                            seq_builder,
                            shown,
                        ),
                        "clear_unpinned" => clear_unpinned_captured(
                            &controls,
                            selected,
                            baseline,
                            compose,
                            seq_builder,
                            shown,
                        ),
                        #[cfg(not(target_arch = "wasm32"))]
                        "launch_browser" => {
                            let start_url = selected_browser_start_url(&controls, selected.get());
                            launch_browser_action(controls, start_url);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        "launch_terminal" => launch_terminal_action(controls),
                        #[cfg(not(target_arch = "wasm32"))]
                        "toggle_adb" => toggle_adb_action(controls),
                        #[cfg(not(target_arch = "wasm32"))]
                        "copy_env" => copy_proxy_env(&controls),
                        #[cfg(not(target_arch = "wasm32"))]
                        "copy_mcp" => copy_mcp_config(&controls),
                        "export_har" => export_har_file(controls, &filter.get()),
                        "export_diagnostics" => export_diagnostics_file(controls),
                        "export_html" => {
                            export_html_file(controls, &filter.get(), ui_lang(langs) == Lang::Zh)
                        }
                        "export_openapi_json" => {
                            export_openapi_file(controls, &filter.get(), false)
                        }
                        "export_openapi_yaml" => export_openapi_file(controls, &filter.get(), true),
                        #[cfg(not(target_arch = "wasm32"))]
                        "copy_openapi_yaml" => copy_openapi_yaml(&controls, &filter.get()),
                        #[cfg(not(target_arch = "wasm32"))]
                        "export_mock_dir" => {
                            export_mock_dir(controls, &filter.get(), profiles.rules)
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        "import_har" => prompt_import_har(controls),
                        #[cfg(not(target_arch = "wasm32"))]
                        "import_rules" => prompt_import_migration(controls, profiles),
                        "sequence_diagram" => begin_sequence_diagram(
                            &controls,
                            rows,
                            seq_builder,
                            compose,
                            selected,
                            shown.phase,
                        ),
                        "toggle_layout" => {
                            let next = if profiles.layout_mode.get() == config::LayoutMode::Split {
                                config::LayoutMode::TopBottom
                            } else {
                                config::LayoutMode::Split
                            };
                            profiles.layout_mode.set(next);
                            save_config_or_notify(&controls, PrefName::List, |cfg| {
                                cfg.layout_mode = next;
                            });
                        }
                        "toggle_sidebar" => {
                            let next = !profiles.sidebar_collapsed.get();
                            profiles.sidebar_collapsed.set(next);
                            save_config_or_notify(&controls, PrefName::List, |cfg| {
                                cfg.sidebar_collapsed = next;
                            });
                        }
                        "toggle_pretty" => {
                            let next = !pretty.get();
                            pretty.set(next);
                            shown.pretty.set(next);
                            save_config_or_notify(&controls, PrefName::Pretty, |cfg| {
                                cfg.default_pretty = next;
                            });
                        }
                        "toggle_json_tree" => {
                            let next = !profiles.default_json_tree.get();
                            profiles.default_json_tree.set(next);
                            save_config_or_notify(&controls, PrefName::Pretty, |cfg| {
                                cfg.default_json_tree = next;
                            });
                        }
                        "show_summary" => {
                            selected.set(None);
                            reset_compose(compose);
                            clear_shown(shown);
                        }
                        "theme_system" => set_theme_mode(&controls, config::ThemeMode::System),
                        "theme_light" => set_theme_mode(&controls, config::ThemeMode::Light),
                        "theme_dark" => set_theme_mode(&controls, config::ThemeMode::Dark),
                        "zoom_in" => step_ui_zoom(&controls, config::UI_ZOOM_STEP as i16),
                        "zoom_out" => step_ui_zoom(&controls, -(config::UI_ZOOM_STEP as i16)),
                        "zoom_reset" => reset_ui_zoom(&controls),
                        "open_settings" => open_settings_tab(profiles, SettingsTab::General),
                        "open_shortcuts" => shortcuts_open.set(true),
                        "open_about" => profiles.about_open.set(true),
                        _ => {}
                    }
                });

            overlay()
                .inset_0()
                .flex_col()
                .items_center()
                .pt(px(72.0))
                .px_4()
                .child(backdrop)
                .child(
                    view()
                        .w(move || px((window().width.0 - 32.0).clamp(300.0, 560.0)))
                        .max_h(move || px((window().height.0 - 110.0).max(200.0)))
                        .rounded(px(12.0))
                        .border_1()
                        .border_color(border_strong())
                        .bg(bg_card())
                        .shadow_lg()
                        .overflow_hidden()
                        .child(palette),
                )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn command_palette_items_have_unique_ids_and_reflect_state() {
        let items = build_command_items(
            Lang::Zh,
            true,
            true,
            true,
            true,
            config::LayoutMode::Split,
            false,
            true,
            false,
            2,
        );
        let ids: HashSet<&str> = items.iter().map(|it| it.id.as_str()).collect();
        assert_eq!(ids.len(), items.len(), "command ids must be unique");
        assert!(ids.contains("toggle_capture"));
        assert!(ids.contains("export_openapi_yaml"));
        assert!(ids.contains("toggle_layout"));
        assert!(ids.contains("open_shortcuts"));
        assert!(ids.contains("focus_filter"));
        assert!(ids.contains("replay_selected"));
        for (id, expected_shortcut) in [
            ("clear_all", "mod+k"),
            ("focus_filter", "mod+f"),
            ("replay_selected", "mod+r"),
            ("show_summary", "escape"),
            ("open_settings", "mod+,"),
            ("open_shortcuts", "mod+/"),
        ] {
            let item = items.iter().find(|it| it.id == id).unwrap();
            assert_eq!(item.shortcut.as_deref(), Some(expected_shortcut));
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let adb = items.iter().find(|it| it.id == "toggle_adb").unwrap();
            assert!(adb.label.contains("ADB · 2"), "{}", adb.label);
            assert!(adb.checked);
        }
    }
}
