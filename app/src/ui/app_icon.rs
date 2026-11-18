#[allow(unused_imports)]
use super::*;
#[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
use std::sync::OnceLock;

#[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
fn icon_cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
fn icon_in_flight() -> &'static Mutex<std::collections::HashSet<String>> {
    static IN_FLIGHT: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    IN_FLIGHT.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

pub(crate) fn base_app_name(raw: &str) -> String {
    let name = session::strip_subprocess_suffix(raw);
    match name {
        "Code" | "Code - Insiders" => "Visual Studio Code".to_string(),
        "msedge" => "Microsoft Edge".to_string(),
        "chrome" => "Google Chrome".to_string(),
        _ => name.to_string(),
    }
}

pub(crate) fn fallback_app_glyph(name: &str) -> Glyph {
    let lower = name.trim().to_ascii_lowercase();
    if lower.contains("chrome")
        || lower.contains("edge")
        || lower.contains("safari")
        || lower.contains("firefox")
        || lower.contains("arc")
        || lower.contains("brave")
        || lower.contains("opera")
        || lower.contains("vivaldi")
        || lower.contains("webkit")
    {
        return Glyph::Globe;
    }
    if matches!(
        lower.as_str(),
        "curl"
            | "wget"
            | "http"
            | "httpie"
            | "node"
            | "python"
            | "python3"
            | "ruby"
            | "go"
            | "cargo"
            | "rustc"
            | "git"
            | "ssh"
            | "zsh"
            | "bash"
            | "fish"
    ) || lower.contains("terminal")
        || lower.contains("iterm")
        || lower.contains("warp")
        || lower.contains("alacritty")
        || lower.contains("kitty")
        || lower.contains("postman")
        || lower.contains("insomnia")
    {
        return Glyph::Prompt;
    }
    Glyph::AppWindow
}

#[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
fn trigger_macos_app_icon_lookup(app_name: String) {
    let base = base_app_name(&app_name);
    if base.is_empty() {
        return;
    }
    {
        let cache = icon_cache().lock().unwrap_or_else(|e| e.into_inner());
        if cache.contains_key(&base) {
            return;
        }
    }
    {
        let mut inflight = icon_in_flight().lock().unwrap_or_else(|e| e.into_inner());
        if !inflight.insert(base.clone()) {
            return;
        }
    }
    std::thread::spawn(move || {
        let resolved = extract_macos_app_icon(&base);
        {
            let mut cache = icon_cache().lock().unwrap_or_else(|e| e.into_inner());
            cache.insert(base.clone(), resolved);
        }
        let mut inflight = icon_in_flight().lock().unwrap_or_else(|e| e.into_inner());
        inflight.remove(&base);
    });
}

#[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
fn extract_macos_app_icon(base: &str) -> Option<String> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let mut candidates = vec![
        std::path::PathBuf::from(format!("/Applications/{base}.app")),
        std::path::PathBuf::from(format!("/System/Applications/{base}.app")),
        std::path::PathBuf::from(format!("/System/Applications/Utilities/{base}.app")),
        std::path::PathBuf::from(format!("/Applications/Utilities/{base}.app")),
    ];
    if let Some(home) = home {
        candidates.push(home.join(format!("Applications/{base}.app")));
    }
    let app_dir = candidates.into_iter().find(|p| p.is_dir())?;
    let resources = app_dir.join("Contents/Resources");
    if !resources.is_dir() {
        return None;
    }
    let plist_path = app_dir.join("Contents/Info.plist");
    let icon_file = std::process::Command::new("defaults")
        .arg("read")
        .arg(&plist_path)
        .arg("CFBundleIconFile")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty());

    let mut icns_candidates = Vec::new();
    if let Some(icon) = icon_file {
        let with_ext = if icon.ends_with(".icns") {
            icon
        } else {
            format!("{icon}.icns")
        };
        icns_candidates.push(resources.join(with_ext));
    }
    icns_candidates.push(resources.join("AppIcon.icns"));
    icns_candidates.push(resources.join(format!("{base}.icns")));
    let icns = icns_candidates
        .into_iter()
        .find(|p| p.is_file())
        .or_else(|| {
            std::fs::read_dir(&resources).ok().and_then(|entries| {
                entries
                    .flatten()
                    .map(|e| e.path())
                    .find(|p| p.extension().and_then(|e| e.to_str()) == Some("icns"))
            })
        })?;

    let cache_dir = std::env::temp_dir().join("zniff-app-icons");
    let _ = std::fs::create_dir_all(&cache_dir);
    let safe_name: String = base
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let out_png = cache_dir.join(format!("{safe_name}.png"));
    if out_png.is_file() {
        return Some(out_png.to_string_lossy().into_owned());
    }
    let ok = std::process::Command::new("sips")
        .args(["-s", "format", "png"])
        .arg(&icns)
        .arg("--out")
        .arg(&out_png)
        .args(["-z", "32", "32"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if ok && out_png.is_file() {
        Some(out_png.to_string_lossy().into_owned())
    } else {
        None
    }
}

pub(crate) fn facet_app_or_device_icon(
    entry: ReadSignal<session::FacetEntry>,
    is_active: impl Fn() -> bool + Copy + 'static,
) -> impl IntoElement {
    let icon_path = signal(None::<String>);
    #[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
    let poll_job = stored(None::<Spawned<()>>);
    #[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
    {
        let (kind, label) = entry.with_untracked(|e| (e.kind, e.label.clone()));
        if kind == session::FacetKind::App {
            let base = base_app_name(&label);
            let cached = icon_cache()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&base)
                .cloned();
            match cached {
                Some(path) => icon_path.set(path),
                None => {
                    trigger_macos_app_icon_lookup(label);
                    poll_job.replace(Some(set_interval(Duration::from_millis(80), move || {
                        let hit = icon_cache()
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .get(&base)
                            .cloned();
                        if let Some(path) = hit {
                            if path.is_some() {
                                icon_path.set(path);
                            }
                            poll_job.replace(None);
                        }
                    })));
                }
            }
        }
    }
    when(
        move || icon_path.with(Option::is_some),
        move || {
            image(move || icon_path.get().unwrap_or_default())
                .w(px(14.0))
                .h(px(14.0))
                .rounded(px(3.0))
                .flex_shrink_0()
        },
    )
    .otherwise(move || {
        glyph_px(
            13.0,
            move || {
                entry.with(|e| match e.kind {
                    session::FacetKind::Mobile => Glyph::DeviceMobile,
                    session::FacetKind::Desktop => Glyph::DeviceDesktop,
                    session::FacetKind::Relay => Glyph::Globe,
                    session::FacetKind::App => fallback_app_glyph(&e.label),
                    session::FacetKind::Domain => Glyph::Globe,
                })
            },
            move || {
                if is_active() {
                    color(accent_blue_text())
                } else {
                    color(fg_secondary())
                }
            },
        )
    })
}
