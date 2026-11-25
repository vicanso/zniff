//! Prepares bundled JetBrains Mono NL font faces for UI targets:
//! - Desktop builds (`desktop` feature, non-`wasm32`) zlib-compress the four
//!   `.ttf` faces into `OUT_DIR` as `*.ttf.zz`.
//! - `wasm32` builds emit `ZNIFF_MONO_*` relative paths stamped with a 10-char
//!   SHA-256 content digest (`fonts/<stem>.<digest>.ttf`) so `scripts/postprocess-web.sh`
//!   serves them as separate immutable assets instead of inflating `.wasm`.
//! - Headless server builds (`--no-default-features` on native targets) skip both.

use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use flate2::Compression;
use flate2::write::ZlibEncoder;

const FACES: [(&str, &str); 4] = [
    ("JetBrainsMonoNL-Regular.ttf", "ZNIFF_MONO_REGULAR"),
    ("JetBrainsMonoNL-Medium.ttf", "ZNIFF_MONO_MEDIUM"),
    ("JetBrainsMonoNL-SemiBold.ttf", "ZNIFF_MONO_SEMIBOLD"),
    ("JetBrainsMonoNL-Bold.ttf", "ZNIFF_MONO_BOLD"),
];

fn resolve_git_dir(repo_root: &std::path::Path) -> Option<PathBuf> {
    let dot_git = repo_root.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    if dot_git.is_file() {
        let raw = fs::read_to_string(&dot_git).ok()?;
        let rel = raw.trim().strip_prefix("gitdir:")?.trim();
        let path = PathBuf::from(rel);
        return Some(if path.is_absolute() {
            path
        } else {
            repo_root.join(path)
        });
    }
    None
}

fn emit_git_metadata() {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    if let Some(repo_root) = manifest_dir.parent()
        && let Some(git_dir) = resolve_git_dir(repo_root)
    {
        let git_head = git_dir.join("HEAD");
        if git_head.is_file() {
            println!("cargo:rerun-if-changed={}", git_head.display());
            if let Ok(head_contents) = fs::read_to_string(&git_head)
                && let Some(ref_rel) = head_contents.trim().strip_prefix("ref:")
            {
                let ref_path = git_dir.join(ref_rel.trim());
                if ref_path.is_file() {
                    println!("cargo:rerun-if-changed={}", ref_path.display());
                }
            }
        }
        let packed_refs = git_dir.join("packed-refs");
        if packed_refs.is_file() {
            println!("cargo:rerun-if-changed={}", packed_refs.display());
        }
    }
    let sha = Command::new("git")
        .current_dir(&manifest_dir)
        .args(["rev-parse", "--short=8", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=ZNIFF_GIT_SHA={sha}");

    let year = Command::new("git")
        .current_dir(&manifest_dir)
        .args(["log", "-1", "--format=%cs"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .split('-')
                .next()
                .map(str::to_string)
        })
        .filter(|y| y.len() == 4 && y.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or_else(|| "2026".to_string());
    println!("cargo:rustc-env=ZNIFF_BUILD_YEAR={year}");
}

fn compile_macos_transparent_proxy_bridge() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos")
        || env::var_os("CARGO_FEATURE_DESKTOP").is_none()
        || env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32")
    {
        return;
    }
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let src = manifest_dir
        .ancestors()
        .map(|dir| dir.join("macos/transparent-proxy/host_bridge.m"))
        .find(|path| path.is_file())
        .unwrap_or_else(|| manifest_dir.join("../macos/transparent-proxy/host_bridge.m"));
    println!("cargo:rerun-if-changed={}", src.display());
    if !src.is_file() {
        panic!("missing macOS transparent proxy bridge: {}", src.display());
    }

    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        other => panic!("unsupported macOS target arch: {other:?}"),
    };
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let obj = out_dir.join("zniff_tp_bridge.o");
    let lib = out_dir.join("libzniff_tp_bridge.a");

    let clang_status = Command::new("xcrun")
        .args([
            "clang",
            "-c",
            "-fobjc-arc",
            "-O2",
            "-mmacosx-version-min=12.0",
            "-arch",
            arch,
            "-o",
        ])
        .arg(&obj)
        .arg(&src)
        .status()
        .expect("failed to invoke xcrun clang for host_bridge.m");
    if !clang_status.success() {
        panic!("xcrun clang failed for host_bridge.m: {clang_status}");
    }

    let ar_status = Command::new("ar")
        .arg("rcs")
        .arg(&lib)
        .arg(&obj)
        .status()
        .expect("failed to invoke ar for libzniff_tp_bridge.a");
    if !ar_status.success() {
        panic!("ar failed for libzniff_tp_bridge.a: {ar_status}");
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=zniff_tp_bridge");
    println!("cargo:rustc-link-lib=framework=SystemExtensions");
    println!("cargo:rustc-link-lib=framework=NetworkExtension");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=AppKit");
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    emit_git_metadata();
    compile_macos_transparent_proxy_bridge();
    for (name, _) in FACES {
        println!("cargo:rerun-if-changed=assets/fonts/{name}");
    }

    let has_desktop = env::var_os("CARGO_FEATURE_DESKTOP").is_some();
    let is_wasm = env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32");
    if !has_desktop && !is_wasm {
        return;
    }

    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    // `nsh package --target web` stages the crate under `.nsh/rust/app-crates/<name>`
    // without copying `assets/`; walk ancestors to locate the real `assets/fonts`.
    let fonts_dir = manifest_dir
        .ancestors()
        .map(|dir| dir.join("assets/fonts"))
        .find(|dir| dir.join(FACES[0].0).is_file())
        .unwrap_or_else(|| manifest_dir.join("assets/fonts"));

    if is_wasm {
        use sha2::{Digest, Sha256};
        for (name, env_key) in FACES {
            let src = fonts_dir.join(name);
            let raw = fs::read(&src)
                .unwrap_or_else(|err| panic!("failed to read font {}: {err}", src.display()));
            let digest = Sha256::digest(&raw);
            let hex10: String = digest[..5].iter().map(|b| format!("{b:02x}")).collect();
            let stem = name.strip_suffix(".ttf").unwrap_or(name);
            println!("cargo:rustc-env={env_key}=fonts/{stem}.{hex10}.ttf");
        }
        return;
    }

    for (name, _) in FACES {
        let src = fonts_dir.join(name);
        let raw = fs::read(&src)
            .unwrap_or_else(|err| panic!("failed to read font {}: {err}", src.display()));
        let mut encoder = ZlibEncoder::new(Vec::with_capacity(raw.len() / 2), Compression::best());
        encoder
            .write_all(&raw)
            .expect("zlib compression into Vec cannot fail");
        let compressed = encoder.finish().expect("zlib finish into Vec cannot fail");
        let dst = out_dir.join(format!("{name}.zz"));
        fs::write(&dst, compressed).unwrap_or_else(|err| {
            panic!("failed to write compressed font {}: {err}", dst.display())
        });
    }
}
