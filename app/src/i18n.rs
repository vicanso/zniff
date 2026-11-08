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
    Bypass,
    Rules,
    Upstream,
    Port,
    Lan,
    List,
    Pretty,
    Tunnel,
    Capture,
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
    InstallFailed(String),
    RemoveFailed(String),
    CopiedUrl,
    CopiedCurl {
        binary: bool,
    },
    EmptyCopy(Side),
    CopiedBody {
        side: Side,
        hex: bool,
    },
    EmptyResponse,
    SaveCancelled,
    SaveFailed(String),
    Saved(String),
    FlowGone,
    BreakDone,
    NeedSelection,
    Replayed(u64),
    Repeated {
        id: u64,
        count: u32,
    },
    ReplayFailed(String),
    ExportCancelled,
    ExportFailed(String),
    Exported(String),
    Shared {
        url: String,
        minutes: u64,
        local_only: bool,
    },
    ShareNeedsCapture,
    ShareStopped,
    ImportFailed(String),
    Imported {
        count: usize,
        path: String,
    },
    CopiedEnv,
    CopiedFetch,
    CopiedPython,
    CopiedInterface,
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
    MappedLocal(String),
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
                    PrefName::Bypass => t(lang, "直通名单没有记住:", "Bypass list was not saved:"),
                    PrefName::Rules => t(lang, "规则没有记住:", "Rules were not saved:"),
                    PrefName::Upstream => {
                        t(lang, "上游代理没有记住:", "Upstream proxy was not saved:")
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
                };
                format!("{name} {err}")
            }
            Self::InstallFailed(err) => {
                format!("{} {err}", t(lang, "安装失败:", "Install failed:"))
            }
            Self::RemoveFailed(err) => format!("{} {err}", t(lang, "移除失败:", "Remove failed:")),
            Self::CopiedUrl => t(lang, "已复制 URL", "Copied the URL").into(),
            Self::CopiedCurl { binary } => {
                if *binary {
                    t(
                        lang,
                        "已复制 cURL，二进制 body 未写入命令",
                        "Copied cURL; the binary body was left out",
                    )
                    .into()
                } else {
                    t(lang, "已复制 cURL", "Copied cURL").into()
                }
            }
            Self::EmptyCopy(side) => format!(
                "{} {}",
                side_name(lang, *side),
                t(
                    lang,
                    "body 是空的，已复制空文本",
                    "body is empty; copied an empty string"
                )
            ),
            Self::CopiedBody { side, hex } => {
                let side = side_name(lang, *side);
                if *hex {
                    format!(
                        "{side} {}",
                        t(lang, "body 已按十六进制复制", "body copied as hex")
                    )
                } else {
                    format!("{} {side} body", t(lang, "已复制", "Copied"))
                }
            }
            Self::EmptyResponse => t(lang, "响应 body 是空的", "Response body is empty").into(),
            Self::SaveCancelled => t(lang, "已取消保存", "Save cancelled").into(),
            Self::SaveFailed(err) => format!(
                "{} {err}",
                t(lang, "没有写成文件:", "Could not write the file:")
            ),
            Self::Saved(path) => format!("{} {path}", t(lang, "已保存", "Saved")),
            Self::FlowGone => t(lang, "这条记录已经不在了", "That flow is gone").into(),
            Self::BreakDone => t(lang, "这个断点已经结束", "That break already finished").into(),
            Self::NeedSelection => t(lang, "先选一条流量", "Select a flow first").into(),
            Self::Replayed(id) => format!("{} #{id}", t(lang, "已重放为", "Replayed as")),
            Self::Repeated { id, count } => {
                format!("{} #{id} ×{count}", t(lang, "已批量重放", "Repeated"))
            }
            Self::ReplayFailed(err) => format!("{} {err}", t(lang, "重放失败:", "Replay failed:")),
            Self::ExportCancelled => t(lang, "已取消导出", "Export cancelled").into(),
            Self::ExportFailed(err) => format!("{} {err}", t(lang, "导出失败:", "Export failed:")),
            Self::Exported(path) => format!("{} {path}", t(lang, "已导出", "Exported")),
            Self::Shared {
                url,
                minutes,
                local_only,
            } => {
                let base = match lang {
                    Lang::Zh => format!("已复制分享地址，{minutes} 分钟内有效 · {url}"),
                    Lang::En => format!("Copied the share link, valid for {minutes} min · {url}"),
                };
                if *local_only {
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
            Self::CopiedFetch => t(lang, "已复制 fetch", "Copied fetch").into(),
            Self::CopiedPython => {
                t(lang, "已复制 Python requests", "Copied Python requests").into()
            }
            Self::CopiedInterface => t(
                lang,
                "已复制 TypeScript interface",
                "Copied the TypeScript interface",
            )
            .into(),
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
                Lang::Zh => format!("已导出 {count} 套配置到剪贴板"),
                Lang::En => format!("Exported {count} setups to clipboard"),
            },
            Self::ProfilesImported { count } => match lang {
                Lang::Zh => format!("已导入 {count} 套配置"),
                Lang::En => format!("Imported {count} setups"),
            },
            Self::MappedLocal(target) => match lang {
                Lang::Zh => format!("已添加本地映射规则 · {target}"),
                Lang::En => format!("Added map-local rule · {target}"),
            },
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
    ];
    for (zh, en) in PREFIX {
        if let Some(rest) = text.strip_prefix(zh) {
            return format!("{en}{rest}");
        }
    }
    text.to_string()
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
                .contains("已导出 2 套配置到剪贴板")
        );
        assert_eq!(
            StatusMsg::ProfilesImported { count: 3 }.text(Lang::En),
            "Imported 3 setups"
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
    }
}
