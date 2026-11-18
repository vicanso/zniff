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

fn emit_git_metadata() {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    if let Some(repo_root) = manifest_dir.parent() {
        let git_head = repo_root.join(".git/HEAD");
        if git_head.exists() {
            println!("cargo:rerun-if-changed={}", git_head.display());
        }
    }
    let sha = Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=ZNIFF_GIT_SHA={sha}");

    let year = Command::new("git")
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

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    emit_git_metadata();
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
