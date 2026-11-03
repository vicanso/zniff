//! Window language. `None` in the preference follows the system locale.
//! Captured error strings stay as the proxy wrote them; [`localize_error`]
//! translates the ones the window shows.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

pub fn lang_from_tag(tag: &str) -> Lang {
    let primary = tag
        .trim()
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .trim();
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
    Bypass,
    Rules,
    Upstream,
    Port,
    Lan,
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
    BindFailed { port: u16, err: String },
    Listening { port: u16, via_system_proxy: bool },
    PortUnsaved { base: Box<StatusMsg>, err: String },
    Stopped,
    StoppedRestored,
    StopRestoreFailed(String),
    ProxyOptionUnsaved(String),
    ProxyManual(String),
    ProxyApplyFailed { port: u16, err: String },
    ProxyRestoreFailed(String),
    PrefUnsaved { name: PrefName, err: String },
    InstallFailed(String),
    RemoveFailed(String),
    CopiedUrl,
    CopiedCurl { binary: bool },
    EmptyCopy(Side),
    CopiedBody { side: Side, hex: bool },
    EmptyResponse,
    SaveFailed(String),
    Saved(String),
    FlowGone,
    BreakDone,
    NeedSelection,
    Replayed(u64),
    ReplayFailed(String),
    ExportFailed(String),
    Exported(String),
    ImportFailed(String),
    Imported { count: usize, path: String },
}

impl StatusMsg {
    pub fn text(&self, lang: Lang) -> String {
        match self {
            Self::Idle => t(lang, "未开始", "Idle").into(),
            Self::CaUnavailable(err) => {
                format!("{} {err}", t(lang, "证书目录不可用:", "Certificate directory unavailable:"))
            }
            Self::Starting => t(lang, "正在启动代理…", "Starting the proxy…").into(),
            Self::BindFailed { port, err } => {
                format!("{} {port} {}: {err}", t(lang, "端口", "Port"), t(lang, "没有绑上", "did not bind"))
            }
            Self::Listening { port, via_system_proxy } => {
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
                format!("{} · {} {err}", base.text(lang), t(lang, "端口没有记住:", "Port was not saved:"))
            }
            Self::Stopped => t(lang, "已停止", "Stopped").into(),
            Self::StoppedRestored => {
                t(lang, "已停止，系统代理已恢复", "Stopped, system proxy restored").into()
            }
            Self::StopRestoreFailed(err) => format!(
                "{} {err}",
                t(lang, "代理已停，恢复系统代理失败:", "Proxy stopped, restoring the system proxy failed:")
            ),
            Self::ProxyOptionUnsaved(err) => {
                format!("{} {err}", t(lang, "系统代理选项没有记住:", "System proxy option was not saved:"))
            }
            Self::ProxyManual(reason) => reason.clone(),
            Self::ProxyApplyFailed { port, err } => format!(
                "{} · 127.0.0.1:{port} · {} {err}",
                t(lang, "抓包中", "Capturing"),
                t(lang, "系统代理没有改成:", "system proxy was not changed:")
            ),
            Self::ProxyRestoreFailed(err) => {
                format!("{} {err}", t(lang, "恢复系统代理失败:", "Restoring the system proxy failed:"))
            }
            Self::PrefUnsaved { name, err } => {
                let name = match name {
                    PrefName::Bypass => t(lang, "直通名单没有记住:", "Bypass list was not saved:"),
                    PrefName::Rules => t(lang, "规则没有记住:", "Rules were not saved:"),
                    PrefName::Upstream => t(lang, "上游代理没有记住:", "Upstream proxy was not saved:"),
                    PrefName::Port => t(lang, "端口没有记住:", "Port was not saved:"),
                    PrefName::Lan => t(lang, "局域网开关没有记住:", "LAN switch was not saved:"),
                };
                format!("{name} {err}")
            }
            Self::InstallFailed(err) => format!("{} {err}", t(lang, "安装失败:", "Install failed:")),
            Self::RemoveFailed(err) => format!("{} {err}", t(lang, "移除失败:", "Remove failed:")),
            Self::CopiedUrl => t(lang, "已复制 URL", "Copied the URL").into(),
            Self::CopiedCurl { binary } => {
                if *binary {
                    t(lang, "已复制 cURL，二进制 body 未写入命令", "Copied cURL; the binary body was left out").into()
                } else {
                    t(lang, "已复制 cURL", "Copied cURL").into()
                }
            }
            Self::EmptyCopy(side) => format!(
                "{} {}",
                side_name(lang, *side),
                t(lang, "body 是空的，已复制空文本", "body is empty; copied an empty string")
            ),
            Self::CopiedBody { side, hex } => {
                let side = side_name(lang, *side);
                if *hex {
                    format!("{side} {}", t(lang, "body 已按十六进制复制", "body copied as hex"))
                } else {
                    format!("{} {side} body", t(lang, "已复制", "Copied"))
                }
            }
            Self::EmptyResponse => t(lang, "响应 body 是空的", "Response body is empty").into(),
            Self::SaveFailed(err) => format!("{} {err}", t(lang, "没有写成文件:", "Could not write the file:")),
            Self::Saved(path) => format!("{} {path}", t(lang, "已保存", "Saved")),
            Self::FlowGone => t(lang, "这条记录已经不在了", "That flow is gone").into(),
            Self::BreakDone => t(lang, "这个断点已经结束", "That break already finished").into(),
            Self::NeedSelection => t(lang, "先选一条流量", "Select a flow first").into(),
            Self::Replayed(id) => format!("{} #{id}", t(lang, "已重放为", "Replayed as")),
            Self::ReplayFailed(err) => format!("{} {err}", t(lang, "重放失败:", "Replay failed:")),
            Self::ExportFailed(err) => format!("{} {err}", t(lang, "导出失败:", "Export failed:")),
            Self::Exported(path) => format!("{} {path}", t(lang, "已导出", "Exported")),
            Self::ImportFailed(err) => format!("{} {err}", t(lang, "导入失败:", "Import failed:")),
            Self::Imported { count, path } => format!(
                "{} {count} {} · {path}",
                t(lang, "已导入", "Imported"),
                t(lang, "条", "flows")
            ),
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
        ("不是 TLS，已改为隧道", "Not TLS, tunneled instead"),
        ("读取 ClientHello 超时", "Timed out reading ClientHello"),
        ("连接源站超时，已改为隧道", "Timed out connecting to the origin, tunneled instead"),
        ("客户端 TLS 超时", "Client TLS timed out"),
        ("断点丢弃", "Dropped at the break"),
        ("消息已截断", "Messages truncated"),
    ];
    if let Some((_, en)) = EXACT.iter().find(|(zh, _)| *zh == text) {
        return (*en).to_string();
    }
    const PREFIX: &[(&str, &str)] = &[
        ("读取 ClientHello 失败: ", "Failed to read ClientHello: "),
        ("源站 TLS 失败，已改为隧道: ", "Origin TLS failed, tunneled instead: "),
        ("客户端 TLS 失败: ", "Client TLS failed: "),
    ];
    for (zh, en) in PREFIX {
        if let Some(rest) = text.strip_prefix(zh) {
            return format!("{en}{rest}");
        }
    }
    text.to_string()
}

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
        assert_eq!(StatusMsg::Idle.text(Lang::En), "Idle");
        assert_eq!(StatusMsg::Idle.text(Lang::Zh), "未开始");
        let listening = StatusMsg::Listening { port: 8877, via_system_proxy: false };
        assert!(listening.text(Lang::En).contains("system proxy unchanged"));
        assert_eq!(localize_error(Lang::Zh, "断点丢弃"), "断点丢弃");
        assert_eq!(localize_error(Lang::En, "断点丢弃"), "Dropped at the break");
        assert_eq!(
            localize_error(Lang::En, "客户端 TLS 失败: eof"),
            "Client TLS failed: eof"
        );
        assert_eq!(ws_kind(Lang::En, "文本"), "text");
    }
}
