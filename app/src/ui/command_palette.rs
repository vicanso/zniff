#[allow(unused_imports)]
use super::*;

use nsh_base::{CommandEntry, CommandGroup, CommandItem};

pub(crate) struct CommandPaletteState {
    pub(crate) lang: Lang,
    pub(crate) running: bool,
    pub(crate) proxy_on: bool,
    pub(crate) has_rows: bool,
    pub(crate) has_selection: bool,
    pub(crate) layout_mode: config::LayoutMode,
    pub(crate) sidebar_collapsed: bool,
    pub(crate) pretty: bool,
    pub(crate) default_json_tree: bool,
    pub(crate) adb_count: usize,
    pub(crate) network_preset: Option<NetworkPreset>,
    pub(crate) theme_mode: config::ThemeMode,
    pub(crate) run_hooks: Vec<crate::hook_spec::ScriptHookConfig>,
}

pub(crate) fn build_command_items(state: CommandPaletteState) -> Vec<CommandItem> {
    let CommandPaletteState {
        lang,
        running,
        proxy_on,
        has_rows,
        has_selection,
        layout_mode,
        sidebar_collapsed,
        pretty,
        default_json_tree,
        adb_count,
        network_preset,
        theme_mode,
        run_hooks,
    } = state;
    let g_capture = t(lang, "抓包", "Capture");
    let g_network = t(lang, "网络条件", "Network");
    let g_copy = t(lang, "复制代码片段", "Copy as Code");
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
        CommandItem::new(
            "open_watch",
            t(lang, "打开匹配提醒（Watch）", "Open Watch Alerts"),
        )
        .group(g_capture)
        .keywords([
            "watch", "alert", "notify", "monitor", "提醒", "监控", "告警",
        ]),
        CommandItem::new("new_request", t(lang, "构造请求", "New request"))
            .group(g_capture)
            .keywords(["compose", "request", "curl", "http", "构造", "请求"]),
        CommandItem::new(
            "add_marker",
            t(lang, "插入会话标记", "Insert session marker"),
        )
        .group(g_capture)
        .shortcut("mod+shift+m")
        .keywords(["marker", "mark", "divider", "timeline", "标记", "分隔线"]),
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
            t(lang, "聚焦列表过滤框", "Focus list filter input"),
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
                    "启动免证书隔离浏览器（Chromium）",
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
        items.push(
            CommandItem::new(
                "init_script_hooks",
                t(
                    lang,
                    "初始化脚本勾子目录（zniff-hook.d.ts）",
                    "Initialize script hooks dir (zniff-hook.d.ts)",
                ),
            )
            .group(g_export)
            .keywords(["hook", "script", "tsx", "dts", "脚本", "勾子"]),
        );
        for hook in &run_hooks {
            if hook.enabled && hook.stages.contains(&crate::hook_spec::HookStage::Run) {
                let title = if hook.name.is_empty() {
                    hook.id.as_str()
                } else {
                    hook.name.as_str()
                };
                items.push(
                    CommandItem::new(
                        format!("run_script_hook:{}", hook.id),
                        format!("{}: {title}", t(lang, "执行脚本勾子", "Run script hook")),
                    )
                    .group(g_export)
                    .keywords([
                        SharedString::from("hook"),
                        SharedString::from("script"),
                        SharedString::from("run"),
                        SharedString::from(hook.id.clone()),
                        SharedString::from("脚本"),
                        SharedString::from("勾子"),
                    ]),
                );
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (proxy_on, adb_count, run_hooks);

    for &preset in NetworkPreset::ALL {
        let label = format!("{}: {}", t(lang, "网络条件", "Network"), preset.label(lang));
        items.push(
            CommandItem::new(preset.command_id(), label)
                .group(g_network)
                .checked(network_preset == Some(preset))
                .keywords([
                    "network", "throttle", "3g", "slow", "latency", "offline", "网络", "弱网",
                    "限速", "延迟", "离线",
                ]),
        );
    }

    items.push(
        CommandItem::new(
            "copy_for_ai",
            t(
                lang,
                "复制给 AI（脱敏 Markdown 上下文）",
                "Copy for AI (redacted Markdown)",
            ),
        )
        .group(g_copy)
        .disabled(!has_selection)
        .keywords([
            "ai", "llm", "markdown", "prompt", "copy", "redact", "debug", "复制", "脱敏",
        ]),
    );
    items.push(
        CommandItem::new(
            "copy_httpie",
            t(lang, "复制为 HTTPie 命令", "Copy as HTTPie"),
        )
        .group(g_copy)
        .disabled(!has_selection)
        .keywords(["copy", "httpie", "http", "cli", "复制"]),
    );
    items.push(
        CommandItem::new(
            "copy_powershell",
            t(
                lang,
                "复制为 PowerShell（Invoke-RestMethod）",
                "Copy as PowerShell (Invoke-RestMethod)",
            ),
        )
        .group(g_copy)
        .disabled(!has_selection)
        .keywords([
            "copy",
            "powershell",
            "ps",
            "invoke-restmethod",
            "windows",
            "复制",
        ]),
    );
    items.push(
        CommandItem::new(
            "copy_axios",
            t(lang, "复制为 JS（axios）", "Copy as JS (axios)"),
        )
        .group(g_copy)
        .disabled(!has_selection)
        .keywords(["copy", "axios", "js", "javascript", "node", "复制"]),
    );
    items.push(
        CommandItem::new(
            "copy_kotlin_okhttp",
            t(lang, "复制为 Kotlin（OkHttp）", "Copy as Kotlin (OkHttp)"),
        )
        .group(g_copy)
        .disabled(!has_selection)
        .keywords(["copy", "kotlin", "okhttp", "android", "jvm", "复制"]),
    );
    items.push(
        CommandItem::new(
            "copy_swift_urlsession",
            t(
                lang,
                "复制为 Swift（URLSession）",
                "Copy as Swift (URLSession)",
            ),
        )
        .group(g_copy)
        .disabled(!has_selection)
        .keywords(["copy", "swift", "urlsession", "ios", "macos", "复制"]),
    );

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
    items.push(
        CommandItem::new(
            "export_postman",
            t(
                lang,
                "导出 Postman Collection v2.1",
                "Export Postman Collection v2.1",
            ),
        )
        .group(g_export)
        .keywords(["postman", "collection", "json", "api", "export", "导出"]),
    );
    items.push(
        CommandItem::new(
            "export_bruno",
            t(lang, "导出 Bruno（.bru）", "Export Bruno (.bru)"),
        )
        .group(g_export)
        .keywords(["bruno", "bru", "collection", "api", "export", "导出"]),
    );
    items.push(
        CommandItem::new(
            "export_http",
            t(
                lang,
                "导出 .http（REST Client）",
                "Export .http (REST Client)",
            ),
        )
        .group(g_export)
        .keywords([
            "http",
            "rest",
            "vscode",
            "jetbrains",
            "collection",
            "export",
            "导出",
        ]),
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
        .checked(theme_mode == config::ThemeMode::System)
        .keywords(["theme", "system", "appearance", "主题", "系统"]),
    );
    items.push(
        CommandItem::new("theme_light", t(lang, "主题：浅色模式", "Theme: Light"))
            .group(g_view)
            .checked(theme_mode == config::ThemeMode::Light)
            .keywords(["theme", "light", "appearance", "主题", "浅色"]),
    );
    items.push(
        CommandItem::new("theme_dark", t(lang, "主题：深色模式", "Theme: Dark"))
            .group(g_view)
            .checked(theme_mode == config::ThemeMode::Dark)
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
            t(lang, "重置界面缩放（100%）", "Reset UI zoom (100%)"),
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
    bind_escape_while(
        move || open.get(),
        nsh_core::keymap::When::Always,
        move || open.set(false),
    );
    let dismiss = popover_dismiss_context();
    effect(move || {
        if open.get()
            && let Some(d) = dismiss
        {
            d.dismiss();
        }
    });
    when(
        move || open.get(),
        move || {
            let langs = ui_langs();
            let exec_controls = controls.clone();
            #[cfg(not(target_arch = "wasm32"))]
            let items_controls = controls.clone();
            let backdrop = ui_modal_backdrop(move || open.set(false));

            let palette = nsh_base::command()
                .auto_focus(true)
                .items(move || {
                    let lang = ui_lang(langs);
                    #[cfg(not(target_arch = "wasm32"))]
                    let adb_count = launcher::configured_adb_count();
                    #[cfg(target_arch = "wasm32")]
                    let adb_count = 0usize;
                    let network_preset = if profiles.rules_on.get() {
                        profiles.rule_groups.with(|groups| {
                            active_preset_from_state(detect_network_group_state(groups))
                        })
                    } else {
                        Some(NetworkPreset::None)
                    };
                    let theme_mode = appearance_prefs()
                        .map(|p| p.theme_mode.get())
                        .unwrap_or(config::ThemeMode::System);
                    #[cfg(not(target_arch = "wasm32"))]
                    let run_hooks = items_controls.hub.script_hooks();
                    #[cfg(target_arch = "wasm32")]
                    let run_hooks = Vec::new();
                    build_command_items(CommandPaletteState {
                        lang,
                        running: running.get(),
                        proxy_on: proxy_on.get(),
                        has_rows: !rows.with(Vec::is_empty),
                        has_selection: selected.get().is_some(),
                        layout_mode: profiles.layout_mode.get(),
                        sidebar_collapsed: profiles.sidebar_collapsed.get(),
                        pretty: pretty.get(),
                        default_json_tree: profiles.default_json_tree.get(),
                        adb_count,
                        network_preset,
                        theme_mode,
                        run_hooks,
                    })
                })
                .placeholder(move || {
                    Some(SharedString::from(tr(
                        "输入命令或关键词…（↑↓ 选择，Enter 执行，Esc 关闭）",
                        "Type a command or keyword… (↑↓ navigate, Enter run, Esc close)",
                    )()))
                })
                .search_bar_class(
                    "flex flex-row items-center gap-2 px-3 py-2.5 border-b border-border",
                )
                .search_class("flex-1 text-sm bg-transparent")
                .list_style(|s| s.max_h(px(360.0)))
                .item_class("w-full px-3 py-2 rounded-md focus:bg-accent hover:bg-accent")
                .group_class("px-3 pt-2 pb-1 text-xs font-semibold text-muted-foreground")
                .separator_class("my-1 h-px bg-border")
                .render_group(|group: &CommandGroup| {
                    text(group.heading.clone())
                        .text_size(px(TEXT_XS))
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
                    let check_slot = view()
                        .w(px(14.0))
                        .h(px(14.0))
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .child(if entry.checked {
                            glyph_px(12.0, || Glyph::Check, || color(accent_blue_text()))
                                .into_element()
                        } else {
                            view().into_element()
                        });
                    let row = view()
                        .w_full()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(check_slot)
                        .child(
                            text(entry.label.clone())
                                .flex_1()
                                .number_of_lines(1)
                                .text_size(px(TEXT_SM))
                                .text_color(label_color),
                        );
                    if let Some(shortcut) = entry.shortcut_label.clone() {
                        row.child(
                            mono(shortcut)
                                .text_size(px(TEXT_XS))
                                .px_1p5()
                                .py_0p5()
                                .rounded(px(RADIUS_SM))
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
                                .text_size(px(TEXT_SM))
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
                        "copy_for_ai" => {
                            copy_snippet(&controls, selected.get(), Snippet::AiContext);
                        }
                        "copy_httpie" => {
                            copy_snippet(&controls, selected.get(), Snippet::Httpie);
                        }
                        "copy_powershell" => {
                            copy_snippet(&controls, selected.get(), Snippet::PowerShell);
                        }
                        "copy_axios" => {
                            copy_snippet(&controls, selected.get(), Snippet::Axios);
                        }
                        "copy_kotlin_okhttp" => {
                            copy_snippet(&controls, selected.get(), Snippet::KotlinOkHttp);
                        }
                        "copy_swift_urlsession" => {
                            copy_snippet(&controls, selected.get(), Snippet::SwiftUrlSession);
                        }
                        "export_har" => export_har_file(controls, &filter.get()),
                        "export_diagnostics" => export_diagnostics_file(controls),
                        "export_html" => {
                            export_html_file(controls, &filter.get(), ui_lang(langs) == Lang::Zh)
                        }
                        "export_openapi_json" => {
                            export_openapi_file(controls, &filter.get(), false)
                        }
                        "export_openapi_yaml" => export_openapi_file(controls, &filter.get(), true),
                        "export_postman" => export_collection_file(
                            controls,
                            &filter.get(),
                            collections::CollectionFormat::Postman,
                        ),
                        "export_bruno" => export_collection_file(
                            controls,
                            &filter.get(),
                            collections::CollectionFormat::Bruno,
                        ),
                        "export_http" => export_collection_file(
                            controls,
                            &filter.get(),
                            collections::CollectionFormat::Http,
                        ),
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
                            publish_detail_for_controls(
                                &shown,
                                &controls,
                                selected.get(),
                                ui_lang(langs),
                                baseline.get(),
                            );
                        }
                        "toggle_json_tree" => {
                            let next = !profiles.default_json_tree.get();
                            profiles.default_json_tree.set(next);
                            if next {
                                pretty.set(true);
                                shown.pretty.set(true);
                            }
                            save_config_or_notify(&controls, PrefName::Pretty, |cfg| {
                                cfg.default_json_tree = next;
                                if next {
                                    cfg.default_pretty = true;
                                }
                            });
                            publish_detail_for_controls(
                                &shown,
                                &controls,
                                selected.get(),
                                ui_lang(langs),
                                baseline.get(),
                            );
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
                        "add_marker" => profiles.marker_modal_open.set(true),
                        "open_watch" => profiles.watch_open.set(true),
                        "open_about" => profiles.about_open.set(true),
                        #[cfg(not(target_arch = "wasm32"))]
                        "init_script_hooks" => {
                            let dir = controls.hub.ca.dir().to_path_buf();
                            match crate::hook_runtime::ensure_hook_types(&dir) {
                                Ok(dts) => {
                                    let hooks_path = dts
                                        .parent()
                                        .map(|p| p.display().to_string())
                                        .unwrap_or_else(|| dts.display().to_string());
                                    set_clipboard(hooks_path.clone());
                                    set_status(
                                        &controls,
                                        StatusMsg::ScriptHooksInitialized { path: hooks_path },
                                    );
                                }
                                Err(err) => {
                                    set_status(
                                        &controls,
                                        StatusMsg::ScriptHookFailed(err.to_string()),
                                    );
                                }
                            }
                        }
                        other => {
                            #[cfg(not(target_arch = "wasm32"))]
                            if let Some(hook_id) = other.strip_prefix("run_script_hook:") {
                                let controls_run = controls.clone();
                                let id_owned = hook_id.to_string();
                                let cur_filter = filter.get_untracked();
                                let cur_selected = selected.get_untracked();
                                std::thread::spawn(move || {
                                    let configured = controls_run.hub.script_hooks();
                                    let res = crate::hook_runtime::eval_run_hook_blocking(
                                        controls_run.hub.ca.dir(),
                                        &configured,
                                        controls_run.hub.hook_session_host(),
                                        &id_owned,
                                        &cur_filter,
                                        cur_selected,
                                    );
                                    match res {
                                        Ok(report) => {
                                            if !report.cards.is_empty() {
                                                *controls_run
                                                    .pending_hook_run
                                                    .lock()
                                                    .unwrap_or_else(|err| err.into_inner()) =
                                                    Some(HookRunModalData {
                                                        hook_id: report.hook_id.clone(),
                                                        summary: report.summary.clone(),
                                                        cards: report.cards,
                                                        duration_ms: report.duration_ms,
                                                    });
                                            }
                                            set_status(
                                                &controls_run,
                                                StatusMsg::ScriptHookCompleted {
                                                    id: report.hook_id,
                                                    summary: report.summary,
                                                },
                                            );
                                        }
                                        Err(err) => {
                                            set_status(
                                                &controls_run,
                                                StatusMsg::ScriptHookFailed(err),
                                            );
                                        }
                                    }
                                });
                                return;
                            }
                            if let Some(preset) = NetworkPreset::from_command_id(other) {
                                apply_network_preset(profiles, &controls, preset);
                            }
                        }
                    }
                });

            dialog()
                .animation(nsh_core::ModalAnimation::None)
                .close_on_escape(false)
                .optional_label(tr_opt("命令面板", "Command Palette"))
                .on_cancel(move || open.set(false))
                .child(
                    overlay()
                        .inset_0()
                        .on_cancel(move || open.set(false))
                        .flex_col()
                        .items_center()
                        .pt(px(72.0))
                        .px_4()
                        .child(backdrop)
                        .child(
                            ui_modal_card()
                                .w(move || px((window().width.0 - 32.0).clamp(300.0, 560.0)))
                                .max_h(move || px((window().height.0 - 110.0).max(200.0)))
                                .child(palette),
                        ),
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
        let items = build_command_items(CommandPaletteState {
            lang: Lang::Zh,
            running: true,
            proxy_on: true,
            has_rows: true,
            has_selection: true,
            layout_mode: config::LayoutMode::Split,
            sidebar_collapsed: false,
            pretty: true,
            default_json_tree: false,
            adb_count: 2,
            network_preset: Some(NetworkPreset::Slow3g),
            theme_mode: config::ThemeMode::Dark,
            run_hooks: Vec::new(),
        });
        let ids: HashSet<&str> = items.iter().map(|it| it.id.as_str()).collect();
        assert!(
            items
                .iter()
                .find(|it| it.id == "theme_dark")
                .unwrap()
                .checked
        );
        assert!(
            !items
                .iter()
                .find(|it| it.id == "theme_light")
                .unwrap()
                .checked
        );
        assert_eq!(ids.len(), items.len(), "command ids must be unique");
        assert!(ids.contains("network_none"));
        assert!(ids.contains("network_3g"));
        assert!(ids.contains("network_slow_3g"));
        assert!(ids.contains("network_high_latency"));
        assert!(ids.contains("network_offline"));
        assert!(
            items
                .iter()
                .find(|it| it.id == "network_slow_3g")
                .unwrap()
                .checked
        );
        assert!(ids.contains("toggle_capture"));
        assert!(ids.contains("export_openapi_yaml"));
        assert!(ids.contains("export_postman"));
        assert!(ids.contains("export_bruno"));
        assert!(ids.contains("export_http"));
        assert!(ids.contains("toggle_layout"));
        assert!(ids.contains("open_shortcuts"));
        assert!(ids.contains("focus_filter"));
        assert!(ids.contains("replay_selected"));
        for copy_id in [
            "copy_for_ai",
            "copy_httpie",
            "copy_powershell",
            "copy_axios",
            "copy_kotlin_okhttp",
            "copy_swift_urlsession",
        ] {
            assert!(ids.contains(copy_id), "missing {copy_id}");
            let item = items.iter().find(|it| it.id == copy_id).unwrap();
            assert!(!item.disabled, "{copy_id} should be enabled when selected");
        }
        for (id, expected_shortcut) in [
            ("add_marker", "mod+shift+m"),
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
