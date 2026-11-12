#[allow(unused_imports)]
use super::*;

pub fn initial_window_size() -> (i32, i32) {
    let cfg = saved_config();
    (cfg.window_width as i32, cfg.window_height as i32)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn cli_info_output(args: &[String]) -> Option<String> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Some(format!(
            concat!(
                "zniff {}\n",
                "\n",
                "Usage:\n",
                "  zniff [--data-dir <dir>] [--proxy-auth <user:pass>]         Open desktop window\n",
                "  zniff --web [--web-bind <ip>] [--web-port <port>] [--bind <ip>] [-p|--port <port>]\n",
                "              [--data-dir <dir>] [--proxy-auth <user:pass>]\n",
                "  zniff --relay --token <secret> [--bind <ip>] [-p|--port <port>] [--data-dir <dir>]\n",
                "  zniff --mcp [-p|--port <port>] [--data-dir <dir>]\n",
                "\n",
                "Options:\n",
                "  --data-dir <dir>         Override CA & config directory (env: ZNIFF_DATA_DIR)\n",
                "  --proxy-auth <user:pass> Require Basic auth on capture port (env: ZNIFF_PROXY_AUTH)\n",
                "  -V, --version            Print version and exit\n",
                "  -h, --help               Print help and exit"
            ),
            env!("CARGO_PKG_VERSION"),
        ));
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        return Some(format!("zniff {}", env!("CARGO_PKG_VERSION")));
    }
    None
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn apply_global_cli_flags(args: &[String]) -> std::result::Result<(), String> {
    let mut idx = 0usize;
    while idx < args.len() {
        let arg = args[idx].as_str();
        if let Some(rest) = arg.strip_prefix("--data-dir=") {
            let trimmed = rest.trim();
            if trimmed.is_empty() {
                return Err("缺少数据目录路径 (--data-dir)".to_string());
            }
            ca::set_data_dir_override(trimmed);
        } else if arg == "--data-dir" {
            idx += 1;
            let next = args
                .get(idx)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "缺少数据目录路径 (--data-dir)".to_string())?;
            ca::set_data_dir_override(next);
        } else if let Some(rest) = arg.strip_prefix("--proxy-auth=") {
            config::set_proxy_auth_override(rest);
        } else if arg == "--proxy-auth" {
            idx += 1;
            let next = args
                .get(idx)
                .ok_or_else(|| "缺少代理认证参数 (--proxy-auth <user:pass>)".to_string())?;
            config::set_proxy_auth_override(next);
        }
        idx += 1;
    }
    Ok(())
}

/// Headless modes exit before the desktop window. `--web` serves the operator
/// console and leaves the capture port closed. `--relay` and `--mcp` are separate.
#[cfg(not(target_arch = "wasm32"))]
pub fn maybe_run_cli_mode() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(info) = cli_info_output(&args) {
        println!("{info}");
        return Some(0);
    }
    if let Err(err) = apply_global_cli_flags(&args) {
        eprintln!("{err}");
        return Some(2);
    }
    let web = args.iter().any(|arg| arg == "--web");
    let relay_mode = args.iter().any(|arg| arg == "--relay");
    let mcp = args.iter().any(|arg| arg == "--mcp");
    if web && (relay_mode || mcp) {
        eprintln!("zniff --web 不能和 --relay 或 --mcp 一起用");
        return Some(2);
    }
    if web {
        let saved = saved_config();
        return Some(web::run_cli(&args, saved.listen_port));
    }
    if relay_mode {
        let saved = saved_config();
        return Some(relay::run_cli(&args, saved.listen_port, &saved.relay_token));
    }
    if !mcp {
        return None;
    }
    let mut port = saved_config().listen_port;
    let mut idx = 0usize;
    while idx < args.len() {
        let arg = args[idx].as_str();
        if let Some(rest) = arg.strip_prefix("--port=") {
            if let Some(parsed) = present::parse_port(rest) {
                port = parsed;
            }
        } else if arg == "--port" || arg == "-p" {
            if let Some(next) = args.get(idx + 1)
                && let Some(parsed) = present::parse_port(next)
            {
                port = parsed;
            }
            idx += 1;
        } else if arg == "--data-dir" || arg == "--proxy-auth" {
            idx += 1;
        }
        idx += 1;
    }
    Some(mcp::run_stdio_bridge(port))
}

pub(crate) struct Controls {
    pub(crate) hub: Arc<Hub>,
    pub(crate) capture: Mutex<Option<Capture>>,
    pub(crate) status: Mutex<StatusMsg>,
    pub(crate) running: Mutex<bool>,
    pub(crate) listen: Mutex<String>,
    pub(crate) listen_port: Mutex<u16>,
    pub(crate) ca_status: Mutex<ca::CaStatus>,
    /// User preference. Off until they turn on 「系统代理」.
    pub(crate) use_system_proxy: Mutex<bool>,
    /// True only after this session's `sysproxy::enable` returned `Applied`.
    pub(crate) proxy_applied: Mutex<bool>,
    /// Start, stop, and the proxy switch share this so they cannot interleave.
    pub(crate) ops: Mutex<()>,
    /// Next 「开始」binds `0.0.0.0` when this is on.
    pub(crate) listen_all: Mutex<bool>,
    pub(crate) update_state: Mutex<UpdateState>,
    pub(crate) redact_export: Mutex<bool>,
    pub(crate) redact_strict: Mutex<bool>,
    pub(crate) redact_fields: Mutex<String>,
    pub(crate) relay_url: Mutex<String>,
    pub(crate) relay_token: Mutex<String>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) relay_push: Mutex<Option<relay::PushHandle>>,
    pub(crate) toast_seq: std::sync::atomic::AtomicU64,
    pub(crate) toast_queue: Mutex<Vec<(u64, StatusMsg)>>,
    pub(crate) config_error: Mutex<Option<String>>,
    pub(crate) config_path: Mutex<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) enum UpdatePhase {
    #[default]
    Idle,
    Checking,
    Downloading {
        version: String,
        received: u64,
        total: u64,
    },
    Installing {
        version: String,
    },
    ReadyToRestart {
        version: String,
    },
    OpenedInstaller {
        version: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct UpdateState {
    pub(crate) phase: UpdatePhase,
    pub(crate) nudge: Option<String>,
    pub(crate) ignored: Option<String>,
}

impl UpdateState {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn from_disk() -> Self {
        Self {
            phase: UpdatePhase::Idle,
            nudge: updater::nudge(),
            ignored: updater::ignored(),
        }
    }

    pub(crate) fn is_busy(&self) -> bool {
        matches!(
            self.phase,
            UpdatePhase::Checking
                | UpdatePhase::Downloading { .. }
                | UpdatePhase::Installing { .. }
        )
    }

    pub(crate) fn has_toolbar_pill(&self) -> bool {
        match &self.phase {
            UpdatePhase::Downloading { .. }
            | UpdatePhase::Installing { .. }
            | UpdatePhase::ReadyToRestart { .. }
            | UpdatePhase::OpenedInstaller { .. } => true,
            UpdatePhase::Idle | UpdatePhase::Checking => self.nudge.is_some(),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ProfileSignals {
    pub(crate) saved: Signal<Vec<config::SavedProfile>>,
    pub(crate) history: Signal<Vec<config::SavedProfile>>,
    pub(crate) name: Signal<String>,
    pub(crate) filter: Signal<String>,
    pub(crate) bypass: Signal<String>,
    pub(crate) rules: Signal<String>,
    pub(crate) upstream: Signal<String>,
    pub(crate) rules_on: Signal<bool>,
    pub(crate) breaks_on: Signal<bool>,
    pub(crate) show_tunnels: Signal<bool>,
    pub(crate) proxy_auth: Signal<String>,
    pub(crate) relay_url: Signal<String>,
    pub(crate) relay_accept: Signal<bool>,
    pub(crate) list_width: Signal<f32>,
    pub(crate) panel_open: Signal<bool>,
    pub(crate) deselect_rev: Signal<u64>,
}

#[derive(Clone, Copy)]
pub(crate) struct Compose {
    pub(crate) composing: Signal<bool>,
    /// True when the open editor is the paused response, not the request.
    pub(crate) response: Signal<bool>,
    pub(crate) method: Signal<String>,
    pub(crate) url: Signal<String>,
    pub(crate) headers: Signal<String>,
    pub(crate) body: Signal<String>,
    pub(crate) status: Signal<String>,
}

#[derive(Clone, PartialEq, Default)]
pub(crate) struct SeqCandidate {
    pub(crate) id: u64,
    pub(crate) method: String,
    pub(crate) host: String,
    pub(crate) path: String,
    pub(crate) status: Option<u16>,
    pub(crate) failed: bool,
    pub(crate) elapsed: String,
    pub(crate) comment: String,
    pub(crate) checked: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct SeqBuilder {
    pub(crate) open: Signal<bool>,
    pub(crate) items: Signal<Vec<SeqCandidate>>,
    pub(crate) include_response: Signal<bool>,
    pub(crate) include_query: Signal<bool>,
    pub(crate) output: Signal<String>,
    pub(crate) diagram: Signal<present::SequenceDiagramData>,
    pub(crate) view_mode: Signal<u8>,
    pub(crate) source_flowchart: Signal<bool>,
    pub(crate) hover_bar: Signal<Option<usize>>,
    pub(crate) redact_open: Signal<bool>,
    pub(crate) redact_export: Signal<bool>,
    pub(crate) redact_strict: Signal<bool>,
    pub(crate) redact_fields: Signal<String>,
    pub(crate) redact_flow_id: Signal<Option<u64>>,
    /// Created with the app, not inside a panel. Virtualized rows can still read
    /// these for a frame after the panel closes, so they must outlive it.
    pub(crate) viewport_w: Signal<f32>,
    pub(crate) track_w: Signal<f32>,
    pub(crate) layout: Memo<(Vec<f32>, f32)>,
    pub(crate) flat_rows: Memo<Vec<DepFlatRow>>,
}

/// Header and rows keep a 2pt selection bar plus `px_3` (12pt). The filter uses the same inset.
pub(crate) const LIST_INSET: f32 = 14.0;
/// Most checked flows the sequence-diagram previews draw. Copy keeps them all.
pub(crate) const SEQ_PREVIEW_CAP: usize = 200;
pub(crate) const LIST_PAD_END: f32 = 12.0;
/// A set bit collapses that detail card. Zero leaves every card open for this window only.
pub(crate) const FOLD_QUERY: u8 = 1;
pub(crate) const FOLD_DIFF: u8 = 1 << 1;
pub(crate) const FOLD_MESSAGES: u8 = 1 << 2;
pub(crate) const FOLD_RES_BODY: u8 = 1 << 3;
pub(crate) const FOLD_RES_HEADERS: u8 = 1 << 4;
pub(crate) const FOLD_REQ_HEADERS: u8 = 1 << 5;
pub(crate) const FOLD_REQ_BODY: u8 = 1 << 6;
/// Toolbar controls share one box. The cert chip is a status, not a button.
pub(crate) const CONTROL_H: f32 = 30.0;
pub(crate) const CONTROL_RADIUS: f32 = 7.0;
pub(crate) const CONTROL_TEXT: f32 = 12.5;

pub(crate) fn ui_langs() -> Signal<Option<Lang>> {
    use_context().expect("language signal")
}

pub(crate) fn ui_lang(langs: Signal<Option<Lang>>) -> Lang {
    langs
        .get()
        .unwrap_or_else(|| lang_from_tag(locale().as_str()))
}

pub(crate) fn tr(zh: &'static str, en: &'static str) -> impl Fn() -> String + 'static {
    let langs = ui_langs();
    move || t(ui_lang(langs), zh, en).to_string()
}
