#[allow(unused_imports)]
use super::*;

#[derive(Clone, Copy)]
pub(crate) enum Snippet {
    Fetch,
    Python,
    RustReqwest,
    GoHttp,
    Types,
    RustStruct,
}

pub(crate) fn copy_proxy_env(controls: &Controls) {
    let port = lock_port(&controls.listen_port);
    let cert = controls.hub.ca.cert_path();
    let auth = controls.hub.proxy_auth_text();
    set_clipboard(present::proxy_env_export_with_auth(
        port,
        &cert.display().to_string(),
        &auth,
    ));
    set_status(controls, StatusMsg::CopiedEnv);
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn copy_mcp_config(controls: &Controls) {
    let port = lock_port(&controls.listen_port);
    set_clipboard(mcp::mcp_config_json(port));
    set_status(controls, StatusMsg::CopiedMcpConfig(port));
}

pub(crate) fn copy_snippet(controls: &Controls, id: Option<u64>, kind: Snippet) {
    let Some(flow) = selected_flow(controls, id) else {
        return;
    };
    match kind {
        Snippet::Fetch => {
            set_clipboard(present::fetch_snippet(&flow));
            set_status(controls, StatusMsg::CopiedFetch);
        }
        Snippet::Python => {
            set_clipboard(present::python_requests(&flow));
            set_status(controls, StatusMsg::CopiedPython);
        }
        Snippet::RustReqwest => {
            set_clipboard(present::rust_reqwest(&flow));
            set_status(controls, StatusMsg::CopiedRustReqwest);
        }
        Snippet::GoHttp => {
            set_clipboard(present::go_http(&flow));
            set_status(controls, StatusMsg::CopiedGoHttp);
        }
        Snippet::Types => match std::str::from_utf8(&flow.response_body)
            .ok()
            .and_then(present::typescript_interface)
        {
            Some(text) => {
                set_clipboard(text);
                set_status(controls, StatusMsg::CopiedInterface);
            }
            None => set_status(controls, StatusMsg::InterfaceUnavailable),
        },
        Snippet::RustStruct => match std::str::from_utf8(&flow.response_body)
            .ok()
            .and_then(present::rust_serde_struct)
        {
            Some(text) => {
                set_clipboard(text);
                set_status(controls, StatusMsg::CopiedRustStruct);
            }
            None => set_status(controls, StatusMsg::InterfaceUnavailable),
        },
    }
}

pub(crate) fn focus_host(controls: &Controls, id: Option<u64>, filter: Signal<String>) {
    let Some(flow) = selected_flow(controls, id) else {
        return;
    };
    let host = flow.host.trim();
    if host.is_empty() {
        return;
    }
    let current = filter.get();
    let existing = split_ui_or_items(&extract_host_or(&current));
    if existing.len() == 1
        && existing[0].eq_ignore_ascii_case(host)
        && current.trim().starts_with("host:")
    {
        return;
    }
    let next_hosts = if existing.is_empty() {
        host.to_string()
    } else if existing.iter().any(|item| item.eq_ignore_ascii_case(host)) {
        existing.join(" | ")
    } else {
        let mut combined = existing;
        combined.push(host.to_string());
        combined.join(" | ")
    };
    filter.set(set_host_or(&current, &next_hosts));
}

pub(crate) fn mute_host(controls: &Controls, id: Option<u64>, filter: Signal<String>) {
    let Some(flow) = selected_flow(controls, id) else {
        return;
    };
    let host = flow.host.trim().to_ascii_lowercase();
    if host.is_empty() || host.split_whitespace().nth(1).is_some() {
        return;
    }
    let current = filter.get();
    let mut existing = split_ui_or_items(&extract_exclude_or(&current));
    if existing.iter().any(|item| item.eq_ignore_ascii_case(&host)) {
        return;
    }
    existing.push(host);
    filter.set(set_exclude_or(&current, &existing.join(" | ")));
}

pub(crate) fn toggle_pin(controls: &Controls, id: Option<u64>) {
    let Some(flow) = selected_flow(controls, id) else {
        return;
    };
    let next = !flow.pinned;
    if controls.hub.set_pinned(flow.id, next) {
        return;
    }
    set_status(
        controls,
        if next {
            StatusMsg::PinLimit
        } else {
            StatusMsg::FlowGone
        },
    );
}

pub(crate) fn set_baseline(
    controls: &Controls,
    id: Option<u64>,
    baseline: Signal<Option<u64>>,
    shown: &Shown,
    lang: Lang,
) {
    let Some(id) = id else {
        set_status(controls, StatusMsg::NeedSelection);
        return;
    };
    if selected_flow(controls, Some(id)).is_none() {
        return;
    }
    if baseline.get() == Some(id) {
        baseline.set(None);
        set_status(controls, StatusMsg::BaselineCleared);
        publish_detail(shown, &controls.hub, Some(id), lang, None);
        return;
    }
    baseline.set(Some(id));
    set_status(controls, StatusMsg::BaselineSet);
    publish_detail(shown, &controls.hub, Some(id), lang, Some(id));
}

pub(crate) fn copy_part(controls: &Controls, id: Option<u64>, part: CopyPart) {
    let Some(flow) = selected_flow(controls, id) else {
        return;
    };
    let (text, note) = match part {
        CopyPart::Url => (flow.url, StatusMsg::CopiedUrl),
        CopyPart::Curl => {
            let binary =
                !flow.request_body.is_empty() && std::str::from_utf8(&flow.request_body).is_err();
            (
                present::curl_command(&flow),
                StatusMsg::CopiedCurl { binary },
            )
        }
        CopyPart::Request => bytes_for_copy(&flow.request_body, Side::Request),
        CopyPart::Response => bytes_for_copy(&flow.response_body, Side::Response),
    };
    set_clipboard(text);
    set_status(controls, note);
}

pub(crate) fn bytes_for_copy(body: &[u8], side: Side) -> (String, StatusMsg) {
    if body.is_empty() {
        return (String::new(), StatusMsg::EmptyCopy(side));
    }
    if let Ok(text) = std::str::from_utf8(body) {
        (text.to_string(), StatusMsg::CopiedBody { side, hex: false })
    } else {
        (hex_dump(body), StatusMsg::CopiedBody { side, hex: true })
    }
}

/// `0001  ` plus `per_row` bytes. A 32-byte row leaves one extra space after
/// the 16th byte. Copy uses `hex_dump`, which has no row numbers.
pub(crate) fn hex_display_rows(body: &[u8], per_row: usize) -> String {
    let per_row = per_row.max(1);
    let mut out = String::with_capacity(body.len() * 4);
    for (index, byte) in body.iter().enumerate() {
        if index % per_row == 0 {
            if index > 0 {
                out.push('\n');
            }
            let row = index / per_row + 1;
            out.push_str(&format!("{row:04}  "));
        } else if per_row > 16 && index % 16 == 0 {
            out.push(' ');
        }
        out.push_str(&format!("{byte:02x} "));
    }
    out
}

/// 12px Menlo is about 7.25px wide. A row is `0001  ` plus `xx ` per byte,
/// and a 32-byte row adds one space between the two halves.
pub(crate) fn hex_line_width(bytes: usize) -> f32 {
    const CHAR: f32 = 7.25;
    let gaps = if bytes > 16 {
        (bytes / 16 - 1) as f32
    } else {
        0.0
    };
    (6.0 + 3.0 * bytes as f32 + gaps) * CHAR
}

/// Use 32 only when that line fits; otherwise stay at 16.
pub(crate) fn hex_row_bytes(width: f32) -> usize {
    if width + 0.01 >= hex_line_width(32) {
        32
    } else {
        16
    }
}

/// `ready_detail` padding (`pl_3p5` + `pr_5`) and the card border, between the pane and the hex area.
pub(crate) const HEX_AREA_CHROME: f32 = 14.0 + 20.0 + 2.0;
/// Horizontal padding of the hex text (`pl_3p5` + `pr_5`).
pub(crate) const HEX_TEXT_PAD: f32 = 14.0 + 20.0;

/// Width the hex area can use inside a detail pane of `pane` points.
pub(crate) fn hex_area_width(pane: f32) -> f32 {
    (pane - HEX_AREA_CHROME).max(0.0)
}

/// Columns once the pane has been shown. Zero stays at 16.
pub(crate) fn hex_cols_for_pane(pane: f32) -> usize {
    if pane <= 0.0 {
        16
    } else {
        hex_row_bytes(hex_area_width(pane) - HEX_TEXT_PAD)
    }
}

/// Keeps raw hex bytes when switching an image response to "Raw" (`pretty == false`),
/// so the hex view can reflow between 16 and 32 bytes per row based on pane width.
pub(crate) fn response_hex_for_view(pretty: bool, preview: &str, response_hex: Vec<u8>) -> Vec<u8> {
    if pretty && !preview.is_empty() {
        Vec::new()
    } else {
        response_hex
    }
}

pub(crate) fn hex_dump(body: &[u8]) -> String {
    let mut out = String::with_capacity(body.len() * 3);
    for (index, byte) in body.iter().enumerate() {
        if index > 0 && index % 16 == 0 {
            out.push('\n');
        }
        out.push_str(&format!("{byte:02x} "));
    }
    out
}

pub(crate) fn spawn_detached<T: 'static>(future: impl std::future::Future<Output = T> + 'static) {
    Owner::detached().run(|| {
        spawn(future);
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn default_save_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok();
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            home.as_deref()
                .map(|h| std::path::PathBuf::from(h).join(".config"))
        });
    let user_dirs =
        config_home.and_then(|dir| std::fs::read_to_string(dir.join("user-dirs.dirs")).ok());
    let cwd = std::env::current_dir().ok();
    default_save_dir_from(home.as_deref(), user_dirs.as_deref(), cwd.as_deref())
}

#[cfg(any(not(target_arch = "wasm32"), test))]
pub(crate) fn default_save_dir_from(
    home: Option<&str>,
    user_dirs_text: Option<&str>,
    cwd: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    if let Some(home) = home {
        if let Some(text) = user_dirs_text
            && let Some(xdg) = xdg_download_dir(home, text)
            && xdg.is_dir()
        {
            return Some(xdg);
        }
        let downloads = std::path::PathBuf::from(home).join("Downloads");
        if downloads.is_dir() {
            return Some(downloads);
        }
        let home_dir = std::path::PathBuf::from(home);
        if home_dir.is_dir() {
            return Some(home_dir);
        }
    }
    cwd.filter(|dir| dir.is_dir())
        .map(std::path::Path::to_path_buf)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn save_dialog_with_default_dir() -> nsh_core::file_dialog::SaveDialog {
    let dialog = nsh_core::file_dialog::SaveDialog::new();
    match default_save_dir() {
        Some(dir) => dialog.directory(dir),
        None => dialog,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn open_dialog_with_default_dir() -> nsh_core::file_dialog::OpenDialog {
    let dialog = nsh_core::file_dialog::OpenDialog::files();
    match default_save_dir() {
        Some(dir) => dialog.directory(dir),
        None => dialog,
    }
}

#[cfg(any(not(target_arch = "wasm32"), test))]
pub(crate) fn write_export_file(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn export_response(controls: Arc<Controls>, id: Option<u64>) {
    let Some(flow) = selected_flow(&controls, id) else {
        return;
    };
    if flow.response_body.is_empty() {
        set_status(&controls, StatusMsg::EmptyResponse);
        return;
    };
    let ext = present::content_type(&flow.response_headers)
        .and_then(present::image_extension)
        .unwrap_or_else(|| body_extension(&flow.response_headers, &flow.response_body));
    let dialog = save_dialog_with_default_dir()
        .file_name(format!("{}.{}", flow.id, ext))
        .filter(nsh_core::file_dialog::FileFilter::new(
            ext.to_ascii_uppercase(),
            [ext],
        ));
    let body = flow.response_body;
    spawn_detached(async move {
        match pick_save_path(dialog).await {
            Ok(Some(path)) => {
                if let Err(err) = write_export_file(&path, &body) {
                    set_status(&controls, StatusMsg::SaveFailed(err.to_string()));
                    return;
                }
                let text = path.display().to_string();
                set_clipboard(text.clone());
                set_status(&controls, StatusMsg::Saved(text));
            }
            Ok(None) => set_status(&controls, StatusMsg::SaveCancelled),
            Err(err) => set_status(&controls, StatusMsg::SaveFailed(err)),
        }
    });
}

/// `XDG_DOWNLOAD_DIR="$HOME/Downloads"` from `user-dirs.dirs`.
#[cfg(any(not(target_arch = "wasm32"), test))]
pub(crate) fn xdg_download_dir(home: &str, text: &str) -> Option<std::path::PathBuf> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "XDG_DOWNLOAD_DIR" {
            continue;
        }
        let mut value = value.trim();
        if value.len() >= 2 {
            let bytes = value.as_bytes();
            let quoted = (bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
                || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'');
            if quoted {
                value = &value[1..value.len() - 1];
            }
        }
        if value.is_empty() {
            return None;
        }
        return Some(std::path::PathBuf::from(value.replace("$HOME", home)));
    }
    None
}

#[cfg(any(not(target_arch = "wasm32"), test))]
pub(crate) fn body_extension(headers: &[(String, String)], body: &[u8]) -> &'static str {
    if let Some(kind) = present::content_type(headers) {
        let kind = kind.to_ascii_lowercase();
        if kind.contains("json") {
            return "json";
        }
        if kind.contains("html") {
            return "html";
        }
        if kind.contains("xml") {
            return "xml";
        }
        if kind.contains("text/") || kind.contains("javascript") {
            return "txt";
        }
    }
    if std::str::from_utf8(body).is_ok() {
        "txt"
    } else {
        "bin"
    }
}

pub(crate) fn selected_flow(controls: &Controls, id: Option<u64>) -> Option<crate::session::Flow> {
    let id = id?;
    let flow = controls.hub.with_session(|session| session.hydrated(id));
    if flow.is_none() {
        set_status(controls, StatusMsg::FlowGone);
    }
    flow
}

pub(crate) fn image_preview_path(id: u64, flow: &crate::session::Flow) -> String {
    if flow.finished_at.is_none() || flow.response_body.is_empty() || flow.response_truncated {
        return String::new();
    }
    let Some(ext) =
        present::content_type(&flow.response_headers).and_then(present::image_extension)
    else {
        return String::new();
    };
    if ext == "svg" {
        return String::new();
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = id;
        if ext == "ico" {
            if let Some(png) = present::ico_to_png(&flow.response_body) {
                return format!("data:image/png;base64,{}", session::base64_encode(&png));
            }
            let Some(sniffed) = present::sniff_raster_extension(&flow.response_body) else {
                return String::new();
            };
            let mime = match sniffed {
                "png" => "image/png",
                "jpg" => "image/jpeg",
                "gif" => "image/gif",
                "webp" => "image/webp",
                _ => return String::new(),
            };
            return format!(
                "data:{mime};base64,{}",
                session::base64_encode(&flow.response_body)
            );
        }
        let mime = match ext {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "bmp" => "image/bmp",
            _ => "application/octet-stream",
        };
        format!(
            "data:{mime};base64,{}",
            session::base64_encode(&flow.response_body)
        )
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let crc = present::crc32(&flow.response_body);
        let dir = ca::data_dir().join("preview");
        if ext == "ico" {
            let png_path = dir.join(format!("{id}-{crc:08x}.png"));
            if let Ok(meta) = std::fs::metadata(&png_path)
                && meta.len() > 0
            {
                return png_path.to_string_lossy().into_owned();
            }
            if let Some(png) = present::ico_to_png(&flow.response_body) {
                if std::fs::create_dir_all(&dir).is_err()
                    || std::fs::write(&png_path, &png).is_err()
                {
                    return String::new();
                }
                return png_path.to_string_lossy().into_owned();
            }
            let Some(sniffed) = present::sniff_raster_extension(&flow.response_body) else {
                return String::new();
            };
            let path = dir.join(format!("{id}-{crc:08x}.{sniffed}"));
            if let Ok(meta) = std::fs::metadata(&path)
                && meta.len() == flow.response_body.len() as u64
            {
                return path.to_string_lossy().into_owned();
            }
            if std::fs::create_dir_all(&dir).is_err()
                || std::fs::write(&path, &flow.response_body).is_err()
            {
                return String::new();
            }
            return path.to_string_lossy().into_owned();
        }
        let path = dir.join(format!("{id}-{crc:08x}.{ext}"));
        if let Ok(meta) = std::fs::metadata(&path)
            && meta.len() == flow.response_body.len() as u64
        {
            return path.to_string_lossy().into_owned();
        }
        if std::fs::create_dir_all(&dir).is_err()
            || std::fs::write(&path, &flow.response_body).is_err()
        {
            return String::new();
        }
        path.to_string_lossy().into_owned()
    }
}

pub(crate) fn lock_bool(slot: &Mutex<bool>) -> bool {
    *slot.lock().unwrap_or_else(|err| err.into_inner())
}

pub(crate) fn set_bool(slot: &Mutex<bool>, value: bool) {
    *slot.lock().unwrap_or_else(|err| err.into_inner()) = value;
}

pub(crate) fn lock_string(slot: &Mutex<String>) -> String {
    slot.lock().unwrap_or_else(|err| err.into_inner()).clone()
}
