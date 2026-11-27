//! Window language. `None` in the preference follows the system locale.
//! Captured error strings stay as the proxy wrote them; [`localize_error`]
//! translates the ones the window shows.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

pub fn lang_from_tag(tag: &str) -> Lang {
    let primary = tag.trim().split(['-', '_']).next().unwrap_or("").trim();
    if primary.eq_ignore_ascii_case("zh") {
        Lang::Zh
    } else {
        Lang::En
    }
}

pub fn parse_pref(text: &str) -> Option<Lang> {
    match text.trim() {
        "zh" => Some(Lang::Zh),
        "en" => Some(Lang::En),
        _ => None,
    }
}

pub fn t(lang: Lang, zh: &'static str, en: &'static str) -> &'static str {
    match lang {
        Lang::Zh => zh,
        Lang::En => en,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefName {
    Filter,
    Bypass,
    Rules,
    Upstream,
    ProxyAuth,
    AllowHosts,
    Profiles,
    Port,
    Lan,
    List,
    Pretty,
    Tunnel,
    Capture,
    Relay,
    Redact,
    Language,
    Window,
    Font,
    Theme,
    UiZoom,
    TransparentProxy,
    Onboarding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Request,
    Response,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StatusMsg {
    Idle,
    CaUnavailable(String),
    Starting,
    BindFailed {
        port: u16,
        err: String,
    },
    Listening {
        port: u16,
        via_system_proxy: bool,
    },
    PortUnsaved {
        base: Box<StatusMsg>,
        err: String,
    },
    Stopped,
    StoppedRestored,
    StopRestoreFailed(String),
    ProxyOptionUnsaved(String),
    ProxyManual(String),
    ProxyApplyFailed {
        port: u16,
        err: String,
    },
    ProxyRestoreFailed(String),
    PrefUnsaved {
        name: PrefName,
        err: String,
    },
    ConfigBroken {
        err: String,
    },
    ConfigReset {
        backup: String,
    },
    CopiedConfigPath(String),
    InstallFailed(String),
    RemoveFailed(String),
    ExternalCaEnabled {
        tool: String,
    },
    ExternalCaRestored,
    ExternalCaSwitchFailed {
        tool: String,
        err: String,
    },
    ExternalCaFallback {
        tool: String,
    },
    CopiedUrl,
    CopiedHeaders {
        zh: &'static str,
        en: &'static str,
    },
    CopiedCurl {
        binary: bool,
    },
    EmptyCopy(Side),
    CopiedBody {
        side: Side,
        hex: bool,
    },
    CopiedJsonValue,
    CopiedMessage,
    CopiedThinking,
    CopiedToolArgs,
    CopiedMarker,
    EmptyResponse,
    SaveCancelled,
    SaveFailed(String),
    Saved(String),
    FlowGone,
    BreakDone,
    NeedSelection,
    NeedUrl,
    Replayed(u64),
    Repeated {
        id: u64,
        count: u32,
    },
    ReplayFailed(String),
    ExportCancelled,
    ExportFailed(String),
    Exported(String),
    MockExported {
        files: usize,
        hosts: usize,
    },
    CopiedOpenApi {
        paths: usize,
        operations: usize,
    },
    Shared {
        url: String,
        minutes: u64,
        local_only: bool,
        relay: bool,
    },
    ShareNeedsCapture,
    ShareStopped,
    RelayBadUrl,
    RelayNeedToken,
    RelayAcceptNeedToken,
    RelayViewOnly,
    RelayPushFailed(String),
    RelayConnected(String),
    ImportFailed(String),
    Imported {
        count: usize,
        path: String,
    },
    CopiedEnv,
    CopiedMcpConfig(u16),
    CopiedFetch,
    CopiedPython,
    CopiedRustReqwest,
    CopiedGoHttp,
    CopiedHttpie,
    CopiedPowerShell,
    CopiedAxios,
    CopiedKotlinOkHttp,
    CopiedSwiftUrlSession,
    CopiedAiContext,
    CopiedInterface,
    CopiedRustStruct,
    InterfaceUnavailable,
    PinLimit,
    BaselineSet,
    BaselineCleared,
    ProfilesExported {
        count: usize,
    },
    ProfilesImported {
        count: usize,
    },
    RulesMigrated {
        source: String,
        converted: usize,
        unmapped: usize,
    },
    ProfileSaved(String),
    ProfileRestored(String),
    ProfileReverted(String),
    MappedLocal(String),
    BreakRuleAdded(String),
    PrimaryFilterPausedForWatch,
    ProtoEditInEditor,
    NetworkPresetApplied {
        name: String,
        re_enabled: bool,
    },
    NetworkPresetCleared {
        kept_custom: bool,
    },
    RuleGroupLimitReached(usize),
    RuleGroupExists(String),
    RuleGroupMissing(String),
    PresetSwitched(String),
    TemplateRuleAdded(String),
    TemplateRuleAlreadyPresent(String),
    CaInstalled,
    CaInstalledNeedsTrust,
    CaRemoved,
    HostAddedToBypass(String),
    #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
    WebConfigUnsaved(String),
    #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
    WebOpFailed(String),
    RulesCleared,
    RuleGroupDeleted(String),
    PresetDeleted(String),
    ComposeHistoryCleared,
    SequenceGenerated {
        count: usize,
    },
    CopiedMermaid,
    CopiedMermaidMarkdown,
    UpdateChecking,
    UpdateUpToDate(String),
    UpdateAvailable(String),
    UpdateCheckFailed(String),
    UpdateDownloading {
        version: String,
        progress: String,
    },
    UpdateInstalling(String),
    UpdateRestarting(String),
    UpdateOpenedInstaller(String),
    UpdateFailed {
        version: String,
        err: String,
    },
    IosSimInstalled {
        count: usize,
        port: u16,
    },
    IosSimNoneBooted,
    IosSimFailed(String),
    BrowserLaunched {
        name: String,
        port: u16,
    },
    BrowserLaunchFailed(String),
    TerminalLaunched {
        name: String,
        port: u16,
    },
    TerminalLaunchFailed(String),
    AdbConfigured {
        count: usize,
        port: u16,
        cert_pushed: bool,
    },
    AdbCleared(usize),
    AdbNoneConnected,
    AdbFailed(String),
    CopiedLinuxCaCmd(String),
    CopiedAndroidCertGuide,
    CopiedSystemInfo,
    #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
    CopiedDiagnostics,
    ClearedFlows {
        count: usize,
    },
    ClearUndone {
        count: usize,
    },
    WatchAlert {
        rule: String,
        target: String,
        count: usize,
    },
    UiZoomChanged(u16),
    ReadOnlyTab,
    TabLimitReached(usize),
    TransparentProxyStarted,
    TransparentProxyNeedsApproval,
    TransparentProxyStopped,
    TransparentProxyFailed(String),
    TransparentProxyNeedsApplications,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    ScriptHooksInitialized {
        path: String,
    },
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    ScriptHookCompleted {
        id: String,
        summary: String,
    },
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    ScriptHookFailed(String),
}

fn port_in_use(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("address already in use")
        || lower.contains("os error 48")
        || lower.contains("os error 10048")
        || lower.contains("only one usage of each socket address")
}

impl StatusMsg {
    pub fn text(&self, lang: Lang) -> String {
        match self {
            Self::Idle => t(lang, "未开始", "Idle").into(),
            Self::CaUnavailable(err) => {
                format!(
                    "{} {err}",
                    t(
                        lang,
                        "证书目录不可用:",
                        "Certificate directory unavailable:"
                    )
                )
            }
            Self::Starting => t(lang, "正在启动代理…", "Starting the proxy…").into(),
            Self::BindFailed { port, err } => {
                if port_in_use(err) {
                    match lang {
                        Lang::Zh => format!(
                            "端口 {port} 已被占用。多半是另一个 zniff 窗口还在抓包。关掉那个窗口，或在底栏改一个端口再按开始。"
                        ),
                        Lang::En => format!(
                            "Port {port} is already in use. Another zniff window is probably still capturing. Close that window, or change the port in the footer and press Start."
                        ),
                    }
                } else {
                    format!(
                        "{} {port} {}: {err}",
                        t(lang, "端口", "Port"),
                        t(lang, "没有绑上", "did not bind")
                    )
                }
            }
            Self::Listening {
                port,
                via_system_proxy,
            } => {
                if *via_system_proxy {
                    format!("{} · 127.0.0.1:{port}", t(lang, "抓包中", "Capturing"))
                } else {
                    format!(
                        "{} · 127.0.0.1:{port} · {}",
                        t(lang, "抓包中", "Capturing"),
                        t(lang, "未改系统代理", "system proxy unchanged")
                    )
                }
            }
            Self::PortUnsaved { base, err } => {
                format!(
                    "{} · {} {err}",
                    base.text(lang),
                    t(lang, "端口没有记住:", "Port was not saved:")
                )
            }
            Self::Stopped => t(lang, "已停止", "Stopped").into(),
            Self::StoppedRestored => t(
                lang,
                "已停止，系统代理已恢复",
                "Stopped, system proxy restored",
            )
            .into(),
            Self::StopRestoreFailed(err) => format!(
                "{} {err}",
                t(
                    lang,
                    "代理已停，恢复系统代理失败:",
                    "Proxy stopped, restoring the system proxy failed:"
                )
            ),
            Self::ProxyOptionUnsaved(err) => {
                format!(
                    "{} {err}",
                    t(
                        lang,
                        "系统代理选项没有记住:",
                        "System proxy option was not saved:"
                    )
                )
            }
            Self::ProxyManual(reason) => reason.clone(),
            Self::ProxyApplyFailed { port, err } => format!(
                "{} · 127.0.0.1:{port} · {} {err}",
                t(lang, "抓包中", "Capturing"),
                t(lang, "系统代理没有改成:", "system proxy was not changed:")
            ),
            Self::ProxyRestoreFailed(err) => {
                format!(
                    "{} {err}",
                    t(
                        lang,
                        "恢复系统代理失败:",
                        "Restoring the system proxy failed:"
                    )
                )
            }
            Self::PrefUnsaved { name, err } => {
                let name = match name {
                    PrefName::Filter => t(lang, "过滤条件没有记住:", "Filter was not saved:"),
                    PrefName::Bypass => t(lang, "直通名单没有记住:", "Bypass list was not saved:"),
                    PrefName::Rules => t(lang, "规则没有记住:", "Rules were not saved:"),
                    PrefName::Upstream => {
                        t(lang, "上游代理没有记住:", "Upstream proxy was not saved:")
                    }
                    PrefName::ProxyAuth => {
                        t(lang, "代理认证没有记住:", "Proxy auth was not saved:")
                    }
                    PrefName::AllowHosts => {
                        t(lang, "允许域名没有记住:", "Allowed hosts were not saved:")
                    }
                    PrefName::Profiles => {
                        t(lang, "预设没有记住:", "Presets were not saved:")
                    }
                    PrefName::Port => t(lang, "端口没有记住:", "Port was not saved:"),
                    PrefName::Lan => t(lang, "局域网开关没有记住:", "LAN switch was not saved:"),
                    PrefName::List => t(lang, "列表设置没有记住:", "List layout was not saved:"),
                    PrefName::Pretty => t(lang, "默认展示没有记住:", "Default view was not saved:"),
                    PrefName::Tunnel => t(
                        lang,
                        "隧道失败开关没有记住:",
                        "Tunnel failure switch was not saved:",
                    ),
                    PrefName::Capture => {
                        t(lang, "抓包开关没有记住:", "Capture switch was not saved:")
                    }
                    PrefName::Relay => {
                        t(lang, "中继设置没有记住:", "Relay settings were not saved:")
                    }
                    PrefName::Redact => t(
                        lang,
                        "脱敏设置没有记住:",
                        "Redaction settings were not saved:",
                    ),
                    PrefName::Language => {
                        t(lang, "界面语言没有记住:", "Language setting was not saved:")
                    }
                    PrefName::Window => t(lang, "窗口尺寸没有记住:", "Window size was not saved:"),
                    PrefName::Font => t(lang, "字体设置没有记住：", "Font settings were not saved:"),
                    PrefName::Theme => t(lang, "主题模式没有记住:", "Theme mode was not saved:"),
                    PrefName::UiZoom => t(lang, "界面缩放没有记住:", "UI zoom was not saved:"),
                    PrefName::TransparentProxy => t(
                        lang,
                        "透明代理设置没有记住:",
                        "Transparent proxy settings were not saved:",
                    ),
                    PrefName::Onboarding => t(
                        lang,
                        "引导提示状态没有记住:",
                        "Onboarding banner preference was not saved:",
                    ),
                };
                let err = localize_error(lang, err);
                format!("{name} {err}")
            }
            Self::ConfigBroken { err } => match lang {
                Lang::Zh => format!("配置文件格式错误，已使用默认配置（更改不会保存）：{err}"),
                Lang::En => {
                    format!("Config format error, using defaults (changes will not save): {err}")
                }
            },
            Self::ConfigReset { backup } => match lang {
                Lang::Zh => format!("已备份原配置至 {backup} 并重置为当前配置"),
                Lang::En => format!("Backed up broken config to {backup} and reset configuration"),
            },
            Self::CopiedConfigPath(path) => match lang {
                Lang::Zh => format!("已复制配置文件路径 · {path}"),
                Lang::En => format!("Copied config file path · {path}"),
            },
            Self::InstallFailed(err) => {
                format!("{} {err}", t(lang, "安装失败:", "Install failed:"))
            }
            Self::RemoveFailed(err) => format!("{} {err}", t(lang, "移除失败:", "Remove failed:")),
            Self::ExternalCaEnabled { tool } => match lang {
                Lang::Zh => format!("已复用 {tool} 根证书"),
                Lang::En => format!("Now reusing {tool} root CA"),
            },
            Self::ExternalCaRestored => t(
                lang,
                "已切回 zniff 根证书",
                "Switched back to zniff root CA",
            )
            .into(),
            Self::ExternalCaSwitchFailed { tool, err } => match lang {
                Lang::Zh => format!("复用 {tool} 证书失败: {err}"),
                Lang::En => format!("Failed to reuse {tool} CA: {err}"),
            },
            Self::ExternalCaFallback { tool } => match lang {
                Lang::Zh => format!("{tool} 证书不可用，已自动切回 zniff 根证书"),
                Lang::En => format!("{tool} CA unavailable; fell back to zniff root CA"),
            },
            Self::CopiedUrl => t(lang, "已复制 URL", "Copied the URL").into(),
            Self::CopiedHeaders { zh, en } => match lang {
                Lang::Zh => format!("已复制全部{zh}"),
                Lang::En => format!("Copied all {}", en.to_lowercase()),
            },
            Self::CopiedCurl { binary } => {
                if *binary {
                    t(
                        lang,
                        "已复制 cURL，二进制正文未写入命令",
                        "Copied cURL; the binary body was left out",
                    )
                    .into()
                } else {
                    t(lang, "已复制 cURL", "Copied cURL").into()
                }
            }
            Self::EmptyCopy(side) => match lang {
                Lang::Zh => format!("{}正文为空，已复制空文本", side_name(lang, *side)),
                Lang::En => {
                    format!("{} body is empty; copied an empty string", side_name(lang, *side))
                }
            },
            Self::CopiedBody { side, hex } => {
                let side = side_name(lang, *side);
                match (lang, *hex) {
                    (Lang::Zh, true) => format!("{side}正文已按十六进制复制"),
                    (Lang::Zh, false) => format!("已复制{side}正文"),
                    (Lang::En, true) => format!("{side} body copied as hex"),
                    (Lang::En, false) => format!("Copied {} body", side.to_ascii_lowercase()),
                }
            }
            Self::CopiedJsonValue => t(lang, "已复制 JSON 值", "Copied JSON value").into(),
            Self::CopiedMessage => t(lang, "已复制消息", "Copied message").into(),
            Self::CopiedThinking => t(lang, "已复制思考过程", "Copied thinking").into(),
            Self::CopiedToolArgs => t(lang, "已复制工具参数", "Copied tool arguments").into(),
            Self::CopiedMarker => t(lang, "已复制标记名称", "Copied marker label").into(),
            Self::EmptyResponse => t(lang, "响应正文为空", "Response body is empty").into(),
            Self::SaveCancelled => t(lang, "已取消保存", "Save cancelled").into(),
            Self::SaveFailed(err) => format!(
                "{} {}",
                t(lang, "没有写成文件:", "Could not write the file:"),
                localize_error(lang, err)
            ),
            Self::Saved(path) => format!("{} {path}", t(lang, "已保存", "Saved")),
            Self::FlowGone => t(lang, "这条记录已经不在了", "That flow is gone").into(),
            Self::BreakDone => t(lang, "这个断点已经结束", "That break already finished").into(),
            Self::NeedSelection => t(lang, "先选一条流量", "Select a flow first").into(),
            Self::NeedUrl => t(
                lang,
                "请输入请求 URL（或粘贴 cURL 命令）",
                "Enter a request URL (or paste a cURL command)",
            )
            .into(),
            Self::Replayed(id) => format!("{} #{id}", t(lang, "已重放为", "Replayed as")),
            Self::Repeated { id, count } => {
                format!("{} #{id} ×{count}", t(lang, "已批量重放", "Repeated"))
            }
            Self::ReplayFailed(err) => format!(
                "{} {}",
                t(lang, "重放失败:", "Replay failed:"),
                localize_error(lang, err)
            ),
            Self::ExportCancelled => t(lang, "已取消导出", "Export cancelled").into(),
            Self::ExportFailed(err) => format!("{} {err}", t(lang, "导出失败:", "Export failed:")),
            Self::Exported(path) => format!("{} {path}", t(lang, "已导出", "Exported")),
            Self::MockExported { files, hosts } => match lang {
                Lang::Zh => {
                    format!("已生成 {files} 个本地 Mock 文件并添加 {hosts} 条 map-local 规则")
                }
                Lang::En => {
                    format!("Exported {files} mock file(s) and added {hosts} map-local rule(s)")
                }
            },
            Self::CopiedOpenApi { paths, operations } => match lang {
                Lang::Zh => {
                    format!("已复制 OpenAPI 3.0 YAML · {paths} 个路径 / {operations} 个接口")
                }
                Lang::En => {
                    format!("Copied OpenAPI 3.0 YAML · {paths} path(s), {operations} operation(s)")
                }
            },
            Self::Shared {
                url,
                minutes,
                local_only,
                relay,
            } => {
                let base = match lang {
                    Lang::Zh => format!("已复制分享地址，{minutes} 分钟内有效 · {url}"),
                    Lang::En => format!("Copied the share link, valid for {minutes} min · {url}"),
                };
                if *relay {
                    format!(
                        "{base} · {}",
                        t(
                            lang,
                            "对方打开这个中继地址即可。",
                            "The other person opens this relay address."
                        )
                    )
                } else if *local_only {
                    format!(
                        "{base} · {}",
                        t(
                            lang,
                            "只在本机可打开。打开局域网后，别人才能访问。",
                            "This computer only. Turn on LAN for other people."
                        )
                    )
                } else {
                    base
                }
            }
            Self::RelayBadUrl => t(
                lang,
                "中继地址要写成 http(s)://主机:端口[/前缀]",
                "Relay address must look like http(s)://host:port[/prefix]",
            )
            .into(),
            Self::RelayNeedToken => t(
                lang,
                "先填写中继密钥，再分享",
                "Fill in the relay token before sharing",
            )
            .into(),
            Self::RelayAcceptNeedToken => t(
                lang,
                "先填写中继密钥，再接收上传",
                "Fill in the relay token before accepting uploads",
            )
            .into(),
            Self::RelayViewOnly => t(
                lang,
                "中继来的流量只能查看、过滤和复制",
                "Relay flows can only be viewed, filtered, and copied",
            )
            .into(),
            Self::RelayPushFailed(err) => {
                format!(
                    "{} {}",
                    t(lang, "中继推送失败:", "Relay push failed:"),
                    localize_error(lang, err)
                )
            }
            Self::RelayConnected(url) => {
                format!(
                    "{} · {url}",
                    t(lang, "中继已恢复", "Relay is receiving again")
                )
            }
            Self::ShareNeedsCapture => t(
                lang,
                "先按开始，分享地址才打得开",
                "Press Start first. The share link needs the proxy.",
            )
            .into(),
            Self::ShareStopped => t(lang, "已停止分享", "Sharing stopped").into(),
            Self::ImportFailed(err) => format!("{} {err}", t(lang, "导入失败:", "Import failed:")),
            Self::Imported { count, path } => format!(
                "{} {count} {} · {path}",
                t(lang, "已导入", "Imported"),
                t(lang, "条", "flows")
            ),
            Self::CopiedEnv => t(lang, "已复制代理环境变量", "Copied the proxy environment").into(),
            Self::CopiedMcpConfig(port) => match lang {
                Lang::Zh => format!("已复制 MCP 配置 · http://127.0.0.1:{port}/mcp"),
                Lang::En => format!("Copied MCP config · http://127.0.0.1:{port}/mcp"),
            },
            Self::CopiedFetch => t(lang, "已复制 fetch", "Copied fetch").into(),
            Self::CopiedPython => {
                t(lang, "已复制 Python requests", "Copied Python requests").into()
            }
            Self::CopiedRustReqwest => t(lang, "已复制 Rust reqwest", "Copied Rust reqwest").into(),
            Self::CopiedGoHttp => t(lang, "已复制 Go net/http", "Copied Go net/http").into(),
            Self::CopiedHttpie => t(lang, "已复制 HTTPie", "Copied HTTPie").into(),
            Self::CopiedPowerShell => t(lang, "已复制 PowerShell", "Copied PowerShell").into(),
            Self::CopiedAxios => t(lang, "已复制 axios", "Copied axios").into(),
            Self::CopiedKotlinOkHttp => {
                t(lang, "已复制 Kotlin OkHttp", "Copied Kotlin OkHttp").into()
            }
            Self::CopiedSwiftUrlSession => {
                t(lang, "已复制 Swift URLSession", "Copied Swift URLSession").into()
            }
            Self::CopiedAiContext => t(
                lang,
                "已复制脱敏 AI 上下文 (Markdown)",
                "Copied redacted AI context (Markdown)",
            )
            .into(),
            Self::CopiedInterface => t(
                lang,
                "已复制 TypeScript interface",
                "Copied the TypeScript interface",
            )
            .into(),
            Self::CopiedRustStruct => {
                t(lang, "已复制 Rust 结构体", "Copied the Rust struct").into()
            }
            Self::InterfaceUnavailable => t(
                lang,
                "响应不是 JSON 对象或数组",
                "Response is not a JSON object or array",
            )
            .into(),
            Self::PinLimit => t(lang, "星标已满 32 条", "The star list is full at 32").into(),
            Self::BaselineSet => t(lang, "已设为对比基准", "Set as the comparison baseline").into(),
            Self::BaselineCleared => {
                t(lang, "已取消对比基准", "Cleared the comparison baseline").into()
            }
            Self::ProfilesExported { count } => match lang {
                Lang::Zh => {
                    if *count == 1 {
                        "已导出当前预设到剪贴板".into()
                    } else {
                        format!("已导出 {count} 个预设到剪贴板")
                    }
                }
                Lang::En => {
                    if *count == 1 {
                        "Exported current preset to clipboard".into()
                    } else {
                        format!("Exported {count} presets to clipboard")
                    }
                }
            },
            Self::ProfilesImported { count } => match lang {
                Lang::Zh => format!("已导入 {count} 个预设"),
                Lang::En => format!("Imported {count} presets"),
            },
            Self::RulesMigrated {
                source,
                converted,
                unmapped,
            } => match (lang, *unmapped) {
                (Lang::Zh, 0) => format!("已从 {source} 转换并应用 {converted} 条规则"),
                (Lang::Zh, u) => {
                    format!("已从 {source} 转换 {converted} 条规则（{u} 条未映射已保留为注释）")
                }
                (Lang::En, 0) => format!("Converted {converted} rules from {source}"),
                (Lang::En, u) => {
                    format!(
                        "Converted {converted} rules from {source} ({u} unmapped kept as # comments)"
                    )
                }
            },
            Self::ProfileSaved(name) => match lang {
                Lang::Zh => format!("已保存预设「{name}」"),
                Lang::En => format!("Saved preset \"{name}\""),
            },
            Self::ProfileRestored(name) => match lang {
                Lang::Zh => format!("已恢复历史预设「{name}」"),
                Lang::En => format!("Restored preset \"{name}\" from history"),
            },
            Self::ProfileReverted(name) => match lang {
                Lang::Zh => format!("已丢弃修改，还原预设「{name}」"),
                Lang::En => format!("Discarded changes and restored preset \"{name}\""),
            },
            Self::MappedLocal(target) => match lang {
                Lang::Zh => format!("已添加本地映射规则 · {target}"),
                Lang::En => format!("Added map-local rule · {target}"),
            },
            Self::BreakRuleAdded(target) => match lang {
                Lang::Zh => format!("已添加断点规则: {target}"),
                Lang::En => format!("Added break rule: {target}"),
            },
            Self::PrimaryFilterPausedForWatch => t(
                lang,
                "主过滤与提醒规则冲突，已自动暂停主过滤",
                "Paused primary filter so Watch hits are visible",
            )
            .into(),
            Self::ProtoEditInEditor => t(
                lang,
                "请在上方纯文本编辑器中修改 proto 规则",
                "Edit proto rules in the text editor above",
            )
            .into(),
            Self::CaInstalled => {
                t(lang, "已安装并信任 zniff 根证书", "Installed and trusted zniff root CA").into()
            }
            Self::CaInstalledNeedsTrust => t(
                lang,
                "根证书已写入系统，但尚未被信任；请在钥匙串/证书库设为「始终信任」",
                "Root CA installed, but not yet trusted; set it to Always Trust in Keychain/store",
            )
            .into(),
            Self::CaRemoved => t(
                lang,
                "已从系统移除 zniff 根证书",
                "Removed zniff root CA from system trust",
            )
            .into(),
            Self::HostAddedToBypass(host) => match lang {
                Lang::Zh => format!("已将 {host} 加入直通名单，新连接将直接隧道转发"),
                Lang::En => {
                    format!("Added {host} to bypass list; new connections will tunnel directly")
                }
            },
            Self::WebConfigUnsaved(err) => format!(
                "{} {}",
                t(lang, "服务端配置没有保存:", "Server config was not saved:"),
                localize_error(lang, err)
            ),
            Self::WebOpFailed(err) => format!(
                "{} {}",
                t(lang, "Web 操作失败:", "Web action failed:"),
                localize_error(lang, err)
            ),
            Self::RulesCleared => {
                t(lang, "已清空当前规则内容", "Cleared rules editor").into()
            }
            Self::RuleGroupDeleted(name) => match lang {
                Lang::Zh => format!("已删除规则分组「{name}」"),
                Lang::En => format!("Deleted rule group \"{name}\""),
            },
            Self::PresetDeleted(name) => match lang {
                Lang::Zh => format!("已删除预设「{name}」"),
                Lang::En => format!("Deleted preset \"{name}\""),
            },
            Self::ComposeHistoryCleared => {
                t(lang, "已清空最近构造记录", "Cleared recent compose history").into()
            }
            Self::NetworkPresetApplied { name, re_enabled } => match (lang, re_enabled) {
                (Lang::Zh, true) => format!("已开启规则引擎并应用网络预设 · {name}"),
                (Lang::Zh, false) => format!("已应用网络预设 · {name}"),
                (Lang::En, true) => format!("Enabled rules engine and applied preset · {name}"),
                (Lang::En, false) => format!("Applied network preset · {name}"),
            },
            Self::NetworkPresetCleared { kept_custom } => match (lang, kept_custom) {
                (Lang::Zh, true) => {
                    "已停用自定义 Network 规则分组（保留原规则）".to_string()
                }
                (Lang::Zh, false) => "已恢复正常网络（无限制）".to_string(),
                (Lang::En, true) => "Paused custom Network rule group (rules kept)".to_string(),
                (Lang::En, false) => "Network throttling cleared".to_string(),
            },
            Self::RuleGroupLimitReached(limit) => match lang {
                Lang::Zh => format!("最多支持 {limit} 个规则分组"),
                Lang::En => format!("Maximum {limit} rule groups reached"),
            },
            Self::RuleGroupExists(name) => match lang {
                Lang::Zh => format!("规则分组「{name}」已存在，已切换"),
                Lang::En => format!("Switched to existing rule group \"{name}\""),
            },
            Self::RuleGroupMissing(name) => match lang {
                Lang::Zh => format!("规则分组「{name}」已被删除"),
                Lang::En => format!("Rule group \"{name}\" was deleted"),
            },
            Self::PresetSwitched(name) => match lang {
                Lang::Zh => format!("已切换预设「{name}」"),
                Lang::En => format!("Switched to preset \"{name}\""),
            },
            Self::TemplateRuleAdded(line) => match lang {
                Lang::Zh => format!("已添加规则 · {line}"),
                Lang::En => format!("Added rule · {line}"),
            },
            Self::TemplateRuleAlreadyPresent(line) => match lang {
                Lang::Zh => format!("规则已存在 · {line}"),
                Lang::En => format!("Rule already present · {line}"),
            },
            Self::SequenceGenerated { count } => match lang {
                Lang::Zh => format!("已生成 {count} 条流量的时序图并复制 Mermaid"),
                Lang::En => {
                    format!("Generated sequence diagram for {count} flows and copied Mermaid")
                }
            },
            Self::CopiedMermaid => t(
                lang,
                "已复制 Mermaid 时序图",
                "Copied Mermaid sequence diagram",
            )
            .into(),
            Self::CopiedMermaidMarkdown => t(
                lang,
                "已复制 Markdown 时序图",
                "Copied Markdown sequence diagram",
            )
            .into(),
            Self::UpdateChecking => t(lang, "正在检查更新…", "Checking for updates…").into(),
            Self::UpdateUpToDate(ver) => match lang {
                Lang::Zh => format!("当前已是最新版本 v{ver}"),
                Lang::En => format!("Already up to date (v{ver})"),
            },
            Self::UpdateAvailable(ver) => match lang {
                Lang::Zh => format!("发现新版本 {ver}，可在顶栏或更多菜单中点击升级"),
                Lang::En => {
                    format!("New version {ver} available; click Upgrade in toolbar or More")
                }
            },
            Self::UpdateCheckFailed(err) => match lang {
                Lang::Zh => format!("检查更新失败: {err}"),
                Lang::En => format!("Update check failed: {err}"),
            },
            Self::UpdateDownloading { version, progress } => match lang {
                Lang::Zh => format!("正在下载 {version} ({progress})…"),
                Lang::En => format!("Downloading {version} ({progress})…"),
            },
            Self::UpdateInstalling(ver) => match lang {
                Lang::Zh => format!("正在安装 {ver}…"),
                Lang::En => format!("Installing {ver}…"),
            },
            Self::UpdateRestarting(ver) => match lang {
                Lang::Zh => format!("已安装 {ver}，正在重启应用…"),
                Lang::En => format!("Installed {ver}, restarting…"),
            },
            Self::UpdateOpenedInstaller(ver) => match lang {
                Lang::Zh => format!("已打开 {ver} 安装镜像，拖入 Applications 后重启"),
                Lang::En => format!("Opened {ver} installer; drag to Applications and restart"),
            },
            Self::UpdateFailed { version, err } => match lang {
                Lang::Zh => format!("升级 {version} 失败: {err}"),
                Lang::En => format!("Failed to update to {version}: {err}"),
            },
            Self::IosSimInstalled { count, port } => match (lang, *port) {
                (Lang::Zh, 0) => format!("已向 {count} 台运行中的 iOS 模拟器安装并信任根证书"),
                (Lang::En, 0) => {
                    format!("Installed and trusted root CA on {count} booted iOS simulator(s)")
                }
                (Lang::Zh, port) => format!(
                    "已向 {count} 台运行中的 iOS 模拟器安装并信任根证书，HTTP/HTTPS 代理设为 127.0.0.1:{port}"
                ),
                (Lang::En, port) => format!(
                    "Installed and trusted the root CA on {count} booted iOS simulator(s); HTTP/HTTPS proxy is 127.0.0.1:{port}"
                ),
            },
            Self::IosSimNoneBooted => t(
                lang,
                "未检测到运行中的 iOS 模拟器，请先启动 Simulator",
                "No booted iOS Simulator found; launch Simulator first",
            )
            .into(),
            Self::IosSimFailed(err) => match lang {
                Lang::Zh => format!("iOS 模拟器配置失败: {err}"),
                Lang::En => format!("iOS Simulator setup failed: {err}"),
            },
            Self::BrowserLaunched { name, port } => match lang {
                Lang::Zh => {
                    format!("已启动隔离 {name}，代理 127.0.0.1:{port}（免改系统代理与证书）")
                }
                Lang::En => format!(
                    "Launched isolated {name} via 127.0.0.1:{port} (no OS proxy/CA changes)"
                ),
            },
            Self::BrowserLaunchFailed(err) => match lang {
                Lang::Zh => format!("启动隔离浏览器失败: {err}"),
                Lang::En => format!("Failed to launch isolated browser: {err}"),
            },
            Self::TerminalLaunched { name, port } => match lang {
                Lang::Zh => format!("已在 {name} 中打开预注入代理终端（127.0.0.1:{port}）"),
                Lang::En => format!("Opened proxied {name} terminal (127.0.0.1:{port})"),
            },
            Self::TerminalLaunchFailed(err) => match lang {
                Lang::Zh => format!("启动代理终端失败: {err}"),
                Lang::En => format!("Failed to launch proxied terminal: {err}"),
            },
            Self::AdbConfigured {
                count,
                port,
                cert_pushed,
            } => match (lang, *cert_pushed) {
                (Lang::Zh, true) => format!(
                    "已为 {count} 台 Android 设备配置 adb 反向代理 127.0.0.1:{port}，并推送根证书至 /sdcard/Download/zniff-ca.crt"
                ),
                (Lang::Zh, false) => {
                    format!("已为 {count} 台 Android 设备配置 adb 反向代理 127.0.0.1:{port}")
                }
                (Lang::En, true) => format!(
                    "Configured adb proxy 127.0.0.1:{port} on {count} Android device(s) and pushed CA to /sdcard/Download/zniff-ca.crt"
                ),
                (Lang::En, false) => {
                    format!("Configured adb proxy 127.0.0.1:{port} on {count} Android device(s)")
                }
            },
            Self::AdbCleared(count) => match lang {
                Lang::Zh => format!("已还原 {count} 台 Android 设备的 adb 代理"),
                Lang::En => format!("Cleared adb proxy on {count} Android device(s)"),
            },
            Self::AdbNoneConnected => t(
                lang,
                "未检测到已授权的 Android 设备，请先连接 USB/模拟器并开启调试",
                "No authorized Android device found; connect a device/emulator with USB debugging",
            )
            .into(),
            Self::AdbFailed(err) => match lang {
                Lang::Zh => format!("Android adb 配置失败: {err}"),
                Lang::En => format!("Android adb setup failed: {err}"),
            },
            Self::CopiedLinuxCaCmd(cmd) => match lang {
                Lang::Zh => format!("已复制系统证书安装命令 · {cmd}"),
                Lang::En => format!("Copied system CA install command · {cmd}"),
            },
            Self::CopiedAndroidCertGuide => t(
                lang,
                "已复制 Android 7+ 证书配置",
                "Copied Android 7+ certificate config",
            )
            .into(),
            Self::CopiedSystemInfo => t(lang, "已复制系统与环境信息", "Copied system info").into(),
            Self::CopiedDiagnostics => t(
                lang,
                "已复制诊断包 (JSON)",
                "Copied diagnostics bundle (JSON)",
            )
            .into(),
            Self::ClearedFlows { count } => match lang {
                Lang::Zh => format!("已清空 {count} 条流量"),
                Lang::En if *count == 1 => "Cleared 1 flow".to_string(),
                Lang::En => format!("Cleared {count} flows"),
            },
            Self::ClearUndone { count } => match lang {
                Lang::Zh => format!("已恢复 {count} 条流量"),
                Lang::En if *count == 1 => "Restored 1 flow".to_string(),
                Lang::En => format!("Restored {count} flows"),
            },
            Self::WatchAlert {
                rule,
                target,
                count,
            } => {
                if *count > 1 {
                    format!("{rule} (+{count}) · {target}")
                } else {
                    format!("{rule} · {target}")
                }
            }
            Self::UiZoomChanged(pct) => match lang {
                Lang::Zh => format!("界面缩放 {pct}%"),
                Lang::En => format!("UI zoom {pct}%"),
            },
            Self::ReadOnlyTab => t(
                lang,
                "当前为只读导入标签，请切换回「实时会话」执行此操作",
                "Imported tab is read-only; switch to Live to modify",
            )
            .into(),
            Self::TabLimitReached(limit) => match lang {
                Lang::Zh => format!("最多同时打开 {limit} 个导入标签，请先关闭旧标签"),
                Lang::En => {
                    format!("Up to {limit} imported tabs can be open at once; close one first")
                }
            },
            Self::TransparentProxyStarted => t(
                lang,
                "已开启透明按进程抓包（未改系统代理）",
                "Transparent per-process capture enabled (system proxy unchanged)",
            )
            .into(),
            Self::TransparentProxyNeedsApproval => t(
                lang,
                "请在系统设置 → 隐私与安全性 / 登录项与扩展中允许 zniff 网络扩展",
                "Allow zniff network extension in System Settings → Privacy & Security / Login Items & Extensions",
            )
            .into(),
            Self::TransparentProxyStopped => {
                t(lang, "已关闭透明按进程抓包", "Transparent capture stopped").into()
            }
            Self::TransparentProxyFailed(err) => match lang {
                Lang::Zh => format!("透明按进程抓包失败: {err}"),
                Lang::En => format!("Transparent capture failed: {}", localize_error(lang, err)),
            },
            Self::TransparentProxyNeedsApplications => if cfg!(target_os = "linux") {
                t(
                    lang,
                    "需要 Linux 5.12+ 且启用 cgroup v2 (/sys/fs/cgroup) 才能开启透明按进程抓包",
                    "Linux 5.12+ with cgroup v2 (/sys/fs/cgroup) is required for Transparent Proxy",
                )
            } else {
                t(
                    lang,
                    "请先将 zniff.app 拖入 /Applications 目录后再开启透明按进程抓包",
                    "Move zniff.app into /Applications first to activate Transparent Proxy",
                )
            }
            .into(),
            Self::ScriptHooksInitialized { path } => match lang {
                Lang::Zh => format!("已初始化脚本勾子目录并复制路径 · {path}"),
                Lang::En => format!("Initialized hooks dir & copied path · {path}"),
            },
            Self::ScriptHookCompleted { id, summary } => {
                if summary.trim().is_empty() {
                    match lang {
                        Lang::Zh => format!("脚本勾子 @{id} 执行完成"),
                        Lang::En => format!("Script hook @{id} completed"),
                    }
                } else {
                    format!("@{id} · {summary}")
                }
            }
            Self::ScriptHookFailed(err) => match lang {
                Lang::Zh => format!("脚本勾子执行失败: {err}"),
                Lang::En => format!("Script hook failed: {err}"),
            },
        }
    }

    /// `Some(false)` for a normal/success toast, `Some(true)` for a warning/error
    /// toast, or `None` for persistent status updates (e.g. `Idle`, `Listening`).
    pub fn toast_kind(&self) -> Option<bool> {
        match self {
            Self::ProfilesExported { .. }
            | Self::ProfilesImported { .. }
            | Self::RulesMigrated { .. }
            | Self::ProfileSaved(_)
            | Self::ProfileRestored(_)
            | Self::ProfileReverted(_)
            | Self::Exported(_)
            | Self::MockExported { .. }
            | Self::Imported { .. }
            | Self::Saved(_)
            | Self::CopiedUrl
            | Self::CopiedHeaders { .. }
            | Self::CopiedCurl { .. }
            | Self::CopiedBody { .. }
            | Self::CopiedJsonValue
            | Self::CopiedMessage
            | Self::CopiedThinking
            | Self::CopiedToolArgs
            | Self::CopiedMarker
            | Self::CopiedEnv
            | Self::CopiedMcpConfig(_)
            | Self::CopiedFetch
            | Self::CopiedPython
            | Self::CopiedRustReqwest
            | Self::CopiedGoHttp
            | Self::CopiedHttpie
            | Self::CopiedPowerShell
            | Self::CopiedAxios
            | Self::CopiedKotlinOkHttp
            | Self::CopiedSwiftUrlSession
            | Self::CopiedAiContext
            | Self::CopiedInterface
            | Self::CopiedRustStruct
            | Self::CopiedMermaid
            | Self::CopiedMermaidMarkdown
            | Self::SequenceGenerated { .. }
            | Self::CopiedLinuxCaCmd(_)
            | Self::CopiedAndroidCertGuide
            | Self::Shared { .. }
            | Self::ShareStopped
            | Self::RelayConnected(_)
            | Self::MappedLocal(_)
            | Self::BreakRuleAdded(_)
            | Self::PrimaryFilterPausedForWatch
            | Self::NetworkPresetApplied { .. }
            | Self::NetworkPresetCleared { .. }
            | Self::RuleGroupExists(_)
            | Self::PresetSwitched(_)
            | Self::TemplateRuleAdded(_)
            | Self::CaInstalled
            | Self::CaRemoved
            | Self::HostAddedToBypass(_)
            | Self::RulesCleared
            | Self::RuleGroupDeleted(_)
            | Self::PresetDeleted(_)
            | Self::ComposeHistoryCleared
            | Self::BaselineSet
            | Self::BaselineCleared
            | Self::Replayed(_)
            | Self::Repeated { .. }
            | Self::IosSimInstalled { .. }
            | Self::BrowserLaunched { .. }
            | Self::TerminalLaunched { .. }
            | Self::AdbConfigured { .. }
            | Self::AdbCleared(_)
            | Self::ExternalCaEnabled { .. }
            | Self::ExternalCaRestored
            | Self::ConfigReset { .. }
            | Self::CopiedConfigPath(_)
            | Self::CopiedOpenApi { .. }
            | Self::CopiedSystemInfo
            | Self::CopiedDiagnostics
            | Self::UpdateUpToDate(_)
            | Self::UpdateAvailable(_)
            | Self::ClearedFlows { .. }
            | Self::UiZoomChanged(_)
            | Self::UpdateOpenedInstaller(_)
            | Self::TransparentProxyStarted
            | Self::TransparentProxyStopped
            | Self::ScriptHooksInitialized { .. }
            | Self::ScriptHookCompleted { .. } => Some(false),
            Self::ExportFailed(_)
            | Self::ImportFailed(_)
            | Self::SaveFailed(_)
            | Self::PrefUnsaved { .. }
            | Self::PortUnsaved { .. }
            | Self::ProxyOptionUnsaved(_)
            | Self::WebConfigUnsaved(_)
            | Self::WebOpFailed(_)
            | Self::ProxyManual(_)
            | Self::ReplayFailed(_)
            | Self::InstallFailed(_)
            | Self::RemoveFailed(_)
            | Self::ExternalCaSwitchFailed { .. }
            | Self::ExternalCaFallback { .. }
            | Self::BindFailed { .. }
            | Self::CaUnavailable(_)
            | Self::StopRestoreFailed(_)
            | Self::ProxyApplyFailed { .. }
            | Self::ProxyRestoreFailed(_)
            | Self::EmptyCopy(_)
            | Self::EmptyResponse
            | Self::InterfaceUnavailable
            | Self::PinLimit
            | Self::ShareNeedsCapture
            | Self::RelayBadUrl
            | Self::RelayNeedToken
            | Self::RelayAcceptNeedToken
            | Self::RelayViewOnly
            | Self::RelayPushFailed(_)
            | Self::NeedSelection
            | Self::NeedUrl
            | Self::FlowGone
            | Self::BreakDone
            | Self::IosSimNoneBooted
            | Self::IosSimFailed(_)
            | Self::BrowserLaunchFailed(_)
            | Self::TerminalLaunchFailed(_)
            | Self::AdbNoneConnected
            | Self::AdbFailed(_)
            | Self::UpdateCheckFailed(_)
            | Self::UpdateFailed { .. }
            | Self::WatchAlert { .. }
            | Self::ConfigBroken { .. }
            | Self::ReadOnlyTab
            | Self::TabLimitReached(_)
            | Self::TransparentProxyNeedsApproval
            | Self::TransparentProxyFailed(_)
            | Self::TransparentProxyNeedsApplications
            | Self::CaInstalledNeedsTrust
            | Self::ProtoEditInEditor
            | Self::RuleGroupLimitReached(_)
            | Self::RuleGroupMissing(_)
            | Self::TemplateRuleAlreadyPresent(_)
            | Self::ScriptHookFailed(_) => Some(true),
            _ => None,
        }
    }

    pub fn toast_title(&self, lang: Lang) -> &'static str {
        match self {
            Self::ProfilesExported { .. } | Self::Exported(_) | Self::MockExported { .. } => {
                t(lang, "导出成功", "Export Succeeded")
            }
            Self::ProfilesImported { .. } | Self::Imported { .. } | Self::RulesMigrated { .. } => {
                t(lang, "导入成功", "Import Succeeded")
            }
            Self::ProfileSaved(_)
            | Self::Saved(_)
            | Self::MappedLocal(_)
            | Self::BreakRuleAdded(_) => t(lang, "保存成功", "Saved Successfully"),
            Self::PrimaryFilterPausedForWatch => t(lang, "已暂停主过滤", "Primary Filter Paused"),
            Self::PresetSwitched(_) => t(lang, "已切换预设", "Preset Switched"),
            Self::CaInstalled => t(lang, "证书已安装", "CA Installed"),
            Self::CaInstalledNeedsTrust => t(lang, "需要信任证书", "Trust Required"),
            Self::CaRemoved => t(lang, "证书已移除", "CA Removed"),
            Self::RulesCleared | Self::ComposeHistoryCleared => t(lang, "已清空", "Cleared"),
            Self::RuleGroupDeleted(_) | Self::PresetDeleted(_) => t(lang, "已删除", "Deleted"),
            Self::ProfileRestored(_) => t(lang, "已恢复历史版本", "History Restored"),
            Self::ProfileReverted(_) => t(lang, "已还原预设", "Preset Restored"),
            Self::CopiedUrl
            | Self::CopiedHeaders { .. }
            | Self::CopiedCurl { .. }
            | Self::CopiedBody { .. }
            | Self::CopiedJsonValue
            | Self::CopiedMessage
            | Self::CopiedThinking
            | Self::CopiedToolArgs
            | Self::CopiedMarker
            | Self::CopiedEnv
            | Self::CopiedMcpConfig(_)
            | Self::CopiedFetch
            | Self::CopiedPython
            | Self::CopiedRustReqwest
            | Self::CopiedGoHttp
            | Self::CopiedHttpie
            | Self::CopiedPowerShell
            | Self::CopiedAxios
            | Self::CopiedKotlinOkHttp
            | Self::CopiedSwiftUrlSession
            | Self::CopiedAiContext
            | Self::CopiedInterface
            | Self::CopiedRustStruct
            | Self::CopiedMermaid
            | Self::CopiedMermaidMarkdown
            | Self::SequenceGenerated { .. }
            | Self::CopiedLinuxCaCmd(_)
            | Self::CopiedAndroidCertGuide
            | Self::CopiedConfigPath(_)
            | Self::CopiedOpenApi { .. }
            | Self::CopiedSystemInfo
            | Self::CopiedDiagnostics
            | Self::Shared { .. } => t(lang, "已复制到剪贴板", "Copied to Clipboard"),
            Self::RelayConnected(_) => t(lang, "中继已恢复", "Relay Reconnected"),
            Self::ConfigReset { .. } => t(lang, "配置已重置", "Config Reset"),
            Self::PrefUnsaved { .. }
            | Self::PortUnsaved { .. }
            | Self::ProxyOptionUnsaved(_)
            | Self::WebConfigUnsaved(_) => t(lang, "保存失败", "Save Failed"),
            Self::Replayed(_) | Self::Repeated { .. } => t(lang, "重放已发送", "Replay Sent"),
            Self::UpdateUpToDate(_) | Self::UpdateAvailable(_) => {
                t(lang, "检查更新", "Update Check")
            }
            Self::ClearedFlows { .. } => t(lang, "已清空会话", "Session Cleared"),
            Self::UiZoomChanged(_) => t(lang, "界面缩放", "UI Zoom"),
            Self::NeedUrl => t(lang, "缺少 URL", "Missing URL"),
            Self::WatchAlert { .. } => t(lang, "匹配提醒命中", "Watch Alert"),
            Self::ExportFailed(_)
            | Self::ImportFailed(_)
            | Self::SaveFailed(_)
            | Self::ReplayFailed(_)
            | Self::InstallFailed(_)
            | Self::RemoveFailed(_)
            | Self::ExternalCaSwitchFailed { .. }
            | Self::IosSimFailed(_)
            | Self::BrowserLaunchFailed(_)
            | Self::TerminalLaunchFailed(_)
            | Self::AdbFailed(_)
            | Self::UpdateCheckFailed(_)
            | Self::UpdateFailed { .. }
            | Self::TransparentProxyFailed(_)
            | Self::WebOpFailed(_)
            | Self::ScriptHookFailed(_) => t(lang, "操作失败", "Action Failed"),
            Self::ConfigBroken { .. } => t(lang, "配置错误", "Configuration Error"),
            _ if self.toast_kind() == Some(true) => t(lang, "操作提示", "Notice"),
            _ => t(lang, "操作成功", "Action Completed"),
        }
    }
}

fn side_name(lang: Lang, side: Side) -> &'static str {
    match (lang, side) {
        (Lang::Zh, Side::Request) => "请求",
        (Lang::Zh, Side::Response) => "响应",
        (Lang::En, Side::Request) => "Request",
        (Lang::En, Side::Response) => "Response",
    }
}

pub fn localize_error(lang: Lang, text: &str) -> String {
    if lang == Lang::Zh || text.is_empty() {
        return text.to_string();
    }
    const EXACT: &[(&str, &str)] = &[
        ("直通名单，未解密", "Bypass list, not decrypted"),
        ("证书被拒绝", "Certificate rejected"),
        (
            "证书被拒绝。猜测：客户端不信任 zniff CA，请安装并信任根证书。",
            "Certificate rejected. Guess: the client does not trust the zniff CA. Install and trust the root.",
        ),
        (
            "证书被拒绝。猜测：该客户端多半做了证书钉扎，不接受代理签发的证书。把主机加入直通名单。若还没安装 zniff CA，先安装。",
            "Certificate rejected. Guess: the client likely pins certificates and will not accept the proxy leaf. Add the host to the bypass list. Install the zniff CA first if it is missing.",
        ),
        (
            "证书被拒绝。猜测：先确认系统已信任 zniff CA。若已信任，就是这个客户端自带证书库或做了钉扎，把主机加入直通名单。",
            "Certificate rejected. Guess: confirm the system trusts the zniff CA. If it does, this client has its own store or pins certificates. Add the host to the bypass list.",
        ),
        ("不是 TLS，已改为隧道", "Not TLS, tunneled instead"),
        ("读取 ClientHello 超时", "Timed out reading ClientHello"),
        (
            "连接源站超时，已改为隧道",
            "Timed out connecting to the origin, tunneled instead",
        ),
        (
            "源站证书不在内置根证书里，已改为隧道",
            "Origin certificate is not signed by a built-in root, tunneled instead",
        ),
        (
            "源站证书已过期，已改为隧道",
            "Origin certificate has expired, tunneled instead",
        ),
        (
            "源站证书还没到生效时间，已改为隧道",
            "Origin certificate is not valid yet, tunneled instead",
        ),
        (
            "源站证书域名对不上，已改为隧道",
            "Origin certificate name does not match, tunneled instead",
        ),
        (
            "源站证书已吊销，已改为隧道",
            "Origin certificate is revoked, tunneled instead",
        ),
        (
            "源站证书无效，已改为隧道",
            "Origin certificate is invalid, tunneled instead",
        ),
        (
            "源站不接受客户端的协议，已改为隧道",
            "Origin rejected the client protocol, tunneled instead",
        ),
        (
            "源站没有给出证书，已改为隧道",
            "Origin presented no certificate, tunneled instead",
        ),
        (
            "源站 TLS 版本或算法不兼容，已改为隧道",
            "Origin TLS version or cipher is incompatible, tunneled instead",
        ),
        (
            "源站 TLS 失败，已改为隧道",
            "Origin TLS failed, tunneled instead",
        ),
        ("客户端 TLS 超时", "Client TLS timed out"),
        ("断点丢弃", "Dropped at the break"),
        ("没有这条记录", "Flow not found"),
        ("会话标记不能重放", "Markers cannot be replayed"),
        ("隧道不能重放", "Tunnel flows cannot be replayed"),
        ("中继流量只能查看", "Relay flows are read-only"),
        ("没有连上", "Could not connect to server"),
        ("没有分享地址", "No share URL returned"),
        ("请求失败", "Request failed"),
        ("重放失败", "Replay failed"),
        ("请求没有编码", "Request could not be encoded"),
        ("中继需要口令", "Relay requires a token"),
        ("中继地址无效", "Invalid relay address"),
        ("需要先开始抓包", "Start capturing first"),
        ("未授权 (401)", "Unauthorized (401)"),
        ("禁止访问 (403)", "Forbidden (403)"),
        (
            "允许域名已由启动参数或环境变量锁定",
            "Allowed hosts are locked by CLI flags or environment variables",
        ),
        (
            "当前构建未内嵌系统扩展",
            "This build does not include the embedded system extension",
        ),
        (
            "规则阻断 (连接已断开)",
            "Blocked by rule (connection dropped)",
        ),
        (
            "响应断点跳过，body 超过 1 MB",
            "Response break skipped, body is over 1 MB",
        ),
        ("消息已截断", "Messages truncated"),
    ];
    if let Some((_, en)) = EXACT.iter().find(|(zh, _)| *zh == text) {
        return (*en).to_string();
    }
    if let Some(translated) = localize_attached_guess(text) {
        return translated;
    }
    const CLIENT_CERT_PREFIX: &[(&str, &str)] = &[
        (
            "客户端证书加载失败: ",
            "Failed to load client certificate: ",
        ),
        (
            "客户端证书加载失败，已改为隧道: ",
            "Failed to load client certificate, tunneled instead: ",
        ),
    ];
    for (zh, en) in CLIENT_CERT_PREFIX {
        if let Some(rest) = text.strip_prefix(zh) {
            return format!("{en}{}", localize_client_cert_chain(rest));
        }
    }
    const PREFIX: &[(&str, &str)] = &[
        ("读取 ClientHello 失败: ", "Failed to read ClientHello: "),
        (
            "源站 TLS 失败，已改为隧道: ",
            "Origin TLS failed, tunneled instead: ",
        ),
        ("源站中止握手（", "Origin aborted the handshake ("),
        ("客户端 TLS 失败: ", "Client TLS failed: "),
        (
            "响应断点读 body 失败: ",
            "Failed to read the body for a response break: ",
        ),
        (
            "目标域名未在允许列表中: ",
            "Target host is not in the allow-list: ",
        ),
    ];
    for (zh, en) in PREFIX {
        if let Some(rest) = text.strip_prefix(zh) {
            return format!("{en}{rest}");
        }
    }
    text.to_string()
}

fn localize_client_cert_chain(chain: &str) -> String {
    let normalized = chain
        .replace(
            "私钥已加密（ENCRYPTED PRIVATE KEY），zniff 暂不支持加密私钥；请先解密后再配置（例如：openssl pkcs8 -in encrypted.key -out decrypted.key）",
            "private key is encrypted (ENCRYPTED PRIVATE KEY), which zniff does not support; decrypt it first (e.g. openssl pkcs8 -in encrypted.key -out decrypted.key)",
        )
        .replace(
            "私钥已加密（Proc-Type: 4,ENCRYPTED），zniff 暂不支持加密私钥；请先解密后再配置（例如：openssl rsa -in encrypted.key -out decrypted.key）",
            "private key is encrypted (Proc-Type: 4,ENCRYPTED), which zniff does not support; decrypt it first (e.g. openssl rsa -in encrypted.key -out decrypted.key)",
        );
    normalized
        .split(": ")
        .map(localize_client_cert_segment)
        .collect::<Vec<_>>()
        .join(": ")
}

fn localize_client_cert_segment(seg: &str) -> String {
    if let Some(rest) = seg.strip_prefix("加载源站 mTLS 客户端证书 (") {
        return format!("loading origin mTLS client certificate ({rest}");
    }
    if let Some(rest) = seg.strip_prefix("配置源站 mTLS 客户端证书 (") {
        return format!("configuring origin mTLS client certificate ({rest}");
    }
    if let Some(rest) = seg.strip_prefix("读取客户端证书 ") {
        if let Some((cert_p, key_part)) = rest.split_once("（私钥 ")
            && let Some(key_p) = key_part.strip_suffix('）')
        {
            return format!("reading client certificate {cert_p} (key {key_p})");
        }
        return format!("reading client certificate {rest}");
    }
    if let Some(rest) = seg.strip_prefix("读取证书文件 ") {
        return format!("reading certificate file {rest}");
    }
    if let Some(rest) = seg.strip_prefix("解析证书链 ") {
        return format!("parsing certificate chain {rest}");
    }
    if let Some(rest) = seg.strip_prefix("证书文件 ")
        && let Some(path) = rest.strip_suffix(" 中未找到 CERTIFICATE")
    {
        return format!("no CERTIFICATE block found in {path}");
    }
    if let Some(rest) = seg.strip_prefix("读取私钥文件 ") {
        return format!("reading private key file {rest}");
    }
    if let Some(rest) = seg.strip_prefix("解析私钥 ") {
        return format!("parsing private key {rest}");
    }
    if let Some(rest) = seg.strip_prefix("解析合并 PEM 私钥 ") {
        return format!("parsing combined PEM private key {rest}");
    }
    match seg {
        "PEM 中未找到 CERTIFICATE 块" => "no CERTIFICATE block found in PEM".to_string(),
        "解析证书 PEM Base64" => "invalid Base64 in certificate PEM".to_string(),
        "解析私钥 PEM Base64" => "invalid Base64 in private key PEM".to_string(),
        "CA PEM 里有非法字符" => "invalid character in PEM Base64".to_string(),
        "PEM 中未找到私钥块（PRIVATE KEY / RSA PRIVATE KEY / EC PRIVATE KEY）" => {
            "no private key block found in PEM (expected PRIVATE KEY / RSA PRIVATE KEY / EC PRIVATE KEY)".to_string()
        }
        _ => seg.to_string(),
    }
}

/// `步骤名。猜测：原因`. Certificate guesses are already in the exact table.
fn localize_attached_guess(text: &str) -> Option<String> {
    let (head, guess) = text.split_once("。猜测：")?;
    let head_en = FLOW_STEP
        .iter()
        .find(|(zh, _)| *zh == head)
        .map(|(_, en)| *en);
    let guess_en = FLOW_GUESS
        .iter()
        .find(|(zh, _)| *zh == guess)
        .map(|(_, en)| *en);
    if head_en.is_none() && guess_en.is_none() {
        return None;
    }
    Some(format!(
        "{}. Guess: {}",
        head_en.unwrap_or(head),
        guess_en.unwrap_or(guess)
    ))
}

const FLOW_STEP: &[(&str, &str)] = &[
    ("HTTP/2 上游", "HTTP/2 upstream"),
    ("上游 HTTP/2", "Upstream HTTP/2"),
    ("上游 HTTP/1.1", "Upstream HTTP/1.1"),
    ("上游 HTTP/1.1 未就绪", "Upstream HTTP/1.1 was not ready"),
    (
        "上游 HTTP/1.1 已经关闭",
        "Upstream HTTP/1.1 was already closed",
    ),
    (
        "上游 HTTP/1.1 没有返回",
        "Upstream HTTP/1.1 returned nothing",
    ),
    ("客户端 HTTP/2", "Client HTTP/2"),
    ("客户端 HTTP/1.1", "Client HTTP/1.1"),
    ("连接源站超时", "Connecting to the origin timed out"),
    ("读请求头超时", "Reading request headers timed out"),
    ("连接上游代理", "Connecting to the upstream proxy"),
    ("组装上游 URI", "Building the upstream URI"),
    ("组装断点响应", "Building the break response"),
    ("组装映射请求", "Building the mapped request"),
    ("映射连接", "Map-remote connect"),
    ("映射连接未就绪", "Map-remote connect was not ready"),
    ("映射请求", "Map-remote request"),
    ("SNI", "SNI"),
];

const FLOW_GUESS: &[(&str, &str)] = &[
    (
        "复用的 HTTP/2 连接已经关掉，请求还没送到源站。下一次会重新连接。",
        "the reused HTTP/2 connection was already closed, so the request never reached the origin. The next request opens a new one.",
    ),
    (
        "到源站的 HTTP/1.1 连接已经结束，这条请求没送出去。",
        "the HTTP/1.1 connection to the origin was already finished, so this request was not sent.",
    ),
    (
        "连接已经关掉，请求没送出去。",
        "the connection was already closed, so the request was not sent.",
    ),
    (
        "到源站的发送端已经结束，多半是上一条把连接关掉了。",
        "the sender to the origin is already gone, usually because an earlier request closed it.",
    ),
    (
        "源站要求改用 HTTP/1.1，不接受这条 HTTP/2。",
        "the origin requires HTTP/1.1 and rejected this HTTP/2 request.",
    ),
    (
        "源站拒绝继续在这条 HTTP/2 连接上接收请求。",
        "the origin refused to take more requests on this HTTP/2 connection.",
    ),
    (
        "源站或网络把连接重置了。",
        "the origin or the network reset the connection.",
    ),
    (
        "源站端口没有服务在听。",
        "nothing is listening on the origin port.",
    ),
    (
        "对端在消息没收完时把连接关了。",
        "the peer closed the connection before the message finished.",
    ),
    ("主机名没有解析到地址。", "the hostname did not resolve."),
    (
        "源站没有在时限内接受连接。可能是网络不通、地址解析慢，或源站没有响应。",
        "the origin did not accept the connection in time. The network may be down, name lookup may be slow, or the origin did not answer.",
    ),
    (
        "客户端迟迟没有把请求头发完。",
        "the client did not finish sending the request headers.",
    ),
    (
        "这一步在时限内没有完成。",
        "this step did not finish in time.",
    ),
    (
        "发往源站的 HTTP/2 请求失败，还没有状态码。常见是这条连接已经失效，或源站把流关掉了。",
        "the HTTP/2 request to the origin failed before a status code. The connection was often already dead, or the origin closed the stream.",
    ),
    (
        "和源站建立 HTTP/2 失败。源站可能不接受这个协议，或刚连上就把连接关了。",
        "the HTTP/2 handshake with the origin failed. It may reject the protocol, or it closed the connection right away.",
    ),
    (
        "和源站的 HTTP/1.1 对话中断。",
        "the HTTP/1.1 exchange with the origin stopped.",
    ),
    (
        "请求送出后，源站没有把响应交回来。",
        "the request was sent, but the origin did not return a response.",
    ),
    (
        "和客户端的连接中断。源站那边可能已经开始处理。",
        "the client connection dropped. The origin may already have started handling the request.",
    ),
    (
        "配置里的上游代理没有接通。",
        "the configured upstream proxy did not connect.",
    ),
    (
        "改写后的地址或响应不是合法的 HTTP。",
        "the rewritten target or response is not valid HTTP.",
    ),
    (
        "按转发规则去连目标时失败。",
        "connecting to the map-remote target failed.",
    ),
    (
        "主机名不能当作 TLS 的服务器名。",
        "the hostname cannot be used as a TLS server name.",
    ),
];

pub fn ws_kind(lang: Lang, kind: &str) -> String {
    let en = match kind {
        "文本" => "text",
        "二进制" => "binary",
        "关闭" => "close",
        "续帧" => "continuation",
        "帧" => "frame",
        other => other,
    };
    match lang {
        Lang::Zh => kind.to_string(),
        Lang::En => en.to_string(),
    }
}

pub fn direction_name(lang: Lang, request: bool) -> &'static str {
    match (lang, request) {
        (Lang::Zh, true) => "客户端",
        (Lang::Zh, false) => "源站",
        (Lang::En, true) => "client",
        (Lang::En, false) => "origin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_and_prefs() {
        assert_eq!(lang_from_tag("zh-Hans-CN"), Lang::Zh);
        assert_eq!(lang_from_tag("zh_CN"), Lang::Zh);
        assert_eq!(lang_from_tag("en-US"), Lang::En);
        assert_eq!(lang_from_tag(""), Lang::En);
        assert_eq!(parse_pref(" zh\n"), Some(Lang::Zh));
        assert_eq!(parse_pref("en"), Some(Lang::En));
        assert_eq!(parse_pref(""), None);
    }

    #[test]
    fn status_and_errors_follow_the_language() {
        let busy = StatusMsg::BindFailed {
            port: 8877,
            err: "Address already in use (os error 48)".into(),
        };
        assert!(busy.text(Lang::Zh).contains("另一个 zniff"));
        assert!(busy.text(Lang::En).contains("already in use"));
        assert!(!busy.text(Lang::Zh).contains("os error"));
        let other = StatusMsg::BindFailed {
            port: 8877,
            err: "permission denied".into(),
        };
        assert!(other.text(Lang::Zh).contains("没有绑上"));
        assert_eq!(StatusMsg::Idle.text(Lang::En), "Idle");
        assert_eq!(StatusMsg::Idle.text(Lang::Zh), "未开始");
        assert_eq!(StatusMsg::BaselineCleared.text(Lang::Zh), "已取消对比基准");
        assert_eq!(
            StatusMsg::BaselineCleared.text(Lang::En),
            "Cleared the comparison baseline"
        );
        let listening = StatusMsg::Listening {
            port: 8877,
            via_system_proxy: false,
        };
        assert!(listening.text(Lang::En).contains("system proxy unchanged"));
        assert_eq!(localize_error(Lang::Zh, "断点丢弃"), "断点丢弃");
        assert_eq!(localize_error(Lang::En, "断点丢弃"), "Dropped at the break");
        assert_eq!(
            localize_error(Lang::En, "客户端 TLS 失败: eof"),
            "Client TLS failed: eof"
        );
        let guessed =
            "HTTP/2 上游。猜测：复用的 HTTP/2 连接已经关掉，请求还没送到源站。下一次会重新连接。";
        assert_eq!(localize_error(Lang::Zh, guessed), guessed);
        assert_eq!(
            localize_error(Lang::En, guessed),
            "HTTP/2 upstream. Guess: the reused HTTP/2 connection was already closed, so the request never reached the origin. The next request opens a new one."
        );
        assert_eq!(ws_kind(Lang::En, "文本"), "text");
        assert!(
            StatusMsg::ProfilesExported { count: 2 }
                .text(Lang::Zh)
                .contains("已导出 2 个预设到剪贴板")
        );
        assert_eq!(
            StatusMsg::ProfilesImported { count: 3 }.text(Lang::En),
            "Imported 3 presets"
        );
        assert_eq!(StatusMsg::ExportCancelled.text(Lang::Zh), "已取消导出");
        assert_eq!(
            StatusMsg::ExportCancelled.text(Lang::En),
            "Export cancelled"
        );
        assert_eq!(StatusMsg::SaveCancelled.text(Lang::Zh), "已取消保存");
        assert_eq!(StatusMsg::SaveCancelled.text(Lang::En), "Save cancelled");
        assert_eq!(
            StatusMsg::Repeated { id: 7, count: 5 }.text(Lang::Zh),
            "已批量重放 #7 ×5"
        );
        assert_eq!(
            StatusMsg::Repeated { id: 7, count: 5 }.text(Lang::En),
            "Repeated #7 ×5"
        );
        assert_eq!(
            StatusMsg::SequenceGenerated { count: 4 }.text(Lang::Zh),
            "已生成 4 条流量的时序图并复制 Mermaid"
        );
        assert_eq!(
            StatusMsg::SequenceGenerated { count: 4 }.text(Lang::En),
            "Generated sequence diagram for 4 flows and copied Mermaid"
        );
        assert_eq!(
            StatusMsg::CopiedMermaid.text(Lang::Zh),
            "已复制 Mermaid 时序图"
        );
        assert_eq!(
            StatusMsg::CopiedMermaidMarkdown.text(Lang::En),
            "Copied Markdown sequence diagram"
        );
        assert_eq!(
            StatusMsg::IosSimInstalled {
                count: 2,
                port: 8877
            }
            .text(Lang::Zh),
            "已向 2 台运行中的 iOS 模拟器安装并信任根证书，HTTP/HTTPS 代理设为 127.0.0.1:8877"
        );
        assert_eq!(
            StatusMsg::IosSimInstalled {
                count: 2,
                port: 8877
            }
            .text(Lang::En),
            "Installed and trusted the root CA on 2 booted iOS simulator(s); HTTP/HTTPS proxy is 127.0.0.1:8877"
        );
        assert_eq!(
            StatusMsg::IosSimInstalled { count: 1, port: 0 }.text(Lang::Zh),
            "已向 1 台运行中的 iOS 模拟器安装并信任根证书"
        );
        assert_eq!(
            StatusMsg::IosSimNoneBooted.text(Lang::En),
            "No booted iOS Simulator found; launch Simulator first"
        );
        assert_eq!(
            StatusMsg::CopiedLinuxCaCmd("sudo update-ca-certificates".into()).text(Lang::Zh),
            "已复制系统证书安装命令 · sudo update-ca-certificates"
        );
        assert_eq!(
            StatusMsg::Shared {
                url: "http://10.0.0.1:9/s/abc".into(),
                minutes: 15,
                local_only: false,
                relay: true,
            }
            .text(Lang::Zh),
            "已复制分享地址，15 分钟内有效 · http://10.0.0.1:9/s/abc · 对方打开这个中继地址即可。"
        );
        assert_eq!(
            StatusMsg::RelayPushFailed("HTTP 401".into()).text(Lang::En),
            "Relay push failed: HTTP 401"
        );
        assert_eq!(
            StatusMsg::RelayConnected("http://10.0.0.1:9/s/abc".into()).text(Lang::Zh),
            "中继已恢复 · http://10.0.0.1:9/s/abc"
        );
        assert_eq!(
            StatusMsg::RelayNeedToken.text(Lang::En),
            "Fill in the relay token before sharing"
        );
        assert_eq!(
            StatusMsg::RelayViewOnly.text(Lang::Zh),
            "中继来的流量只能查看、过滤和复制"
        );
        assert_eq!(
            StatusMsg::RelayAcceptNeedToken.text(Lang::En),
            "Fill in the relay token before accepting uploads"
        );
    }

    #[test]
    fn test_config_broken_toast_formatting() {
        let msg = StatusMsg::ConfigBroken {
            err: "invalid TOML syntax at line 2 col 4".to_string(),
        };
        assert_eq!(msg.toast_kind(), Some(true));
        assert_eq!(msg.toast_title(Lang::Zh), "配置错误");
        assert_eq!(msg.toast_title(Lang::En), "Configuration Error");
        let zh = msg.text(Lang::Zh);
        assert!(zh.contains("配置文件格式错误"));
        assert!(zh.contains("invalid TOML syntax at line 2 col 4"));
        let en = msg.text(Lang::En);
        assert!(en.contains("Config format error"));
        assert!(en.contains("invalid TOML syntax at line 2 col 4"));
    }

    #[test]
    fn localize_error_translates_client_cert_errors_and_save_failure_toasts() {
        let cases = [
            "客户端证书加载失败: 加载源站 mTLS 客户端证书 (client.badssl.com): 读取客户端证书 /tmp/c.pem: 解析合并 PEM 私钥 /tmp/c.pem: 私钥已加密（ENCRYPTED PRIVATE KEY），zniff 暂不支持加密私钥；请先解密后再配置（例如：openssl pkcs8 -in encrypted.key -out decrypted.key）",
            "客户端证书加载失败: 加载源站 mTLS 客户端证书 (client.badssl.com): 读取客户端证书 /tmp/c.crt（私钥 /tmp/c.key）: 解析私钥 /tmp/c.key: 私钥已加密（Proc-Type: 4,ENCRYPTED），zniff 暂不支持加密私钥；请先解密后再配置（例如：openssl rsa -in encrypted.key -out decrypted.key）",
            "客户端证书加载失败: 加载源站 mTLS 客户端证书 (client.badssl.com): 读取客户端证书 /tmp/c.pem: 解析合并 PEM 私钥 /tmp/c.pem: PEM 中未找到私钥块（PRIVATE KEY / RSA PRIVATE KEY / EC PRIVATE KEY）",
            "客户端证书加载失败: 加载源站 mTLS 客户端证书 (client.badssl.com): 读取客户端证书 /tmp/c.crt: 证书文件 /tmp/c.crt 中未找到 CERTIFICATE",
            "客户端证书加载失败: 配置源站 mTLS 客户端证书 (mtls.example.com, cert=/tmp/a.crt, key=/tmp/b.key): invalid peer certificate: KeyMismatch",
            "客户端证书加载失败，已改为隧道: 配置源站 mTLS 客户端证书 (mtls.example.com, pem=/tmp/a.pem): invalid peer certificate: KeyMismatch",
        ];
        for raw in cases {
            let en = localize_error(Lang::En, raw);
            assert!(
                en.starts_with("Failed to load client certificate"),
                "unexpected translation: {en}"
            );
            assert!(
                !en.chars().any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch)),
                "Chinese characters leaked into English error: {en}"
            );
            assert_eq!(localize_error(Lang::Zh, raw), raw);
        }

        let pref_unsaved = StatusMsg::PrefUnsaved {
            name: PrefName::Filter,
            err: "config.toml 格式错误".into(),
        };
        assert_eq!(pref_unsaved.toast_kind(), Some(true));
        assert!(
            pref_unsaved
                .text(Lang::En)
                .contains("Filter was not saved:")
        );

        let port_unsaved = StatusMsg::PortUnsaved {
            base: Box::new(StatusMsg::Listening {
                port: 8877,
                via_system_proxy: false,
            }),
            err: "read-only".into(),
        };
        assert_eq!(port_unsaved.toast_kind(), Some(true));
        assert_eq!(
            StatusMsg::ProxyOptionUnsaved("perm".into()).toast_kind(),
            Some(true)
        );
        assert_eq!(
            StatusMsg::ConfigReset {
                backup: "/tmp/config.toml.broken".into()
            }
            .toast_kind(),
            Some(false)
        );
        assert_eq!(
            StatusMsg::CopiedConfigPath("/tmp/config.toml".into()).toast_kind(),
            Some(false)
        );
    }

    #[test]
    fn cleared_flows_and_undo_messages_are_bilingual() {
        let cleared = StatusMsg::ClearedFlows { count: 5 };
        assert_eq!(cleared.toast_kind(), Some(false));
        assert_eq!(cleared.toast_title(Lang::Zh), "已清空会话");
        assert_eq!(cleared.toast_title(Lang::En), "Session Cleared");
        assert_eq!(cleared.text(Lang::Zh), "已清空 5 条流量");
        assert_eq!(cleared.text(Lang::En), "Cleared 5 flows");
        assert_eq!(
            StatusMsg::ClearedFlows { count: 1 }.text(Lang::En),
            "Cleared 1 flow"
        );

        let undone = StatusMsg::ClearUndone { count: 5 };
        assert_eq!(undone.toast_kind(), None);
        assert_eq!(undone.text(Lang::Zh), "已恢复 5 条流量");
        assert_eq!(undone.text(Lang::En), "Restored 5 flows");
        assert_eq!(
            StatusMsg::ClearUndone { count: 1 }.text(Lang::En),
            "Restored 1 flow"
        );

        let watch = StatusMsg::WatchAlert {
            rule: "API 5xx".into(),
            target: "GET api.x.com/pay 502".into(),
            count: 2,
        };
        assert_eq!(watch.toast_kind(), Some(true));
        assert_eq!(watch.toast_title(Lang::Zh), "匹配提醒命中");
        assert_eq!(watch.toast_title(Lang::En), "Watch Alert");
        assert!(watch.text(Lang::En).contains("(+2)"));

        let bypass_added = StatusMsg::HostAddedToBypass("api.example.com".into());
        assert_eq!(bypass_added.toast_kind(), Some(false));
        assert!(bypass_added.text(Lang::Zh).contains("api.example.com"));
        assert!(bypass_added.text(Lang::En).contains("api.example.com"));

        let web_unsaved = StatusMsg::WebConfigUnsaved("请求失败".into());
        assert_eq!(web_unsaved.toast_kind(), Some(true));
        assert!(web_unsaved.text(Lang::En).contains("Request failed"));

        let web_err = StatusMsg::WebOpFailed("未授权 (401)".into());
        assert_eq!(web_err.toast_kind(), Some(true));
        assert!(web_err.text(Lang::En).contains("Unauthorized (401)"));

        assert_eq!(localize_error(Lang::En, "请求失败"), "Request failed");
        assert_eq!(
            localize_error(Lang::En, "未授权 (401)"),
            "Unauthorized (401)"
        );
        assert_eq!(
            localize_error(Lang::En, "禁止访问 (403)"),
            "Forbidden (403)"
        );

        let copied_diag = StatusMsg::CopiedDiagnostics;
        assert_eq!(copied_diag.toast_kind(), Some(false));
        assert_eq!(copied_diag.text(Lang::Zh), "已复制诊断包 (JSON)");
        assert_eq!(
            copied_diag.text(Lang::En),
            "Copied diagnostics bundle (JSON)"
        );

        let body_ok = StatusMsg::CopiedBody {
            side: Side::Response,
            hex: false,
        };
        assert_eq!(body_ok.text(Lang::Zh), "已复制响应正文");
        assert_eq!(body_ok.text(Lang::En), "Copied response body");
    }
}
