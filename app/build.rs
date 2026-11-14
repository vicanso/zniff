//! Compresses bundled UI font faces into `OUT_DIR` for builds that render the
//! UI (`desktop` or `wasm32`). Headless server builds (`--no-default-features`
//! on native targets) skip embedding the font bytes completely.

use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use flate2::Compression;
use flate2::write::ZlibEncoder;

const FACES: [&str; 4] = [
    "JetBrainsMonoNL-Regular.ttf",
    "JetBrainsMonoNL-Medium.ttf",
    "JetBrainsMonoNL-SemiBold.ttf",
    "JetBrainsMonoNL-Bold.ttf",
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    for name in FACES {
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

    for name in FACES {
        let src = manifest_dir.join("assets/fonts").join(name);
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
