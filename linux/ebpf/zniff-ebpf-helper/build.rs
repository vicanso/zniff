use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=ZNIFF_EBPF_OBJ");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let dst = out_dir.join("zniff-ebpf-prog");

    if let Some(obj_path) = env::var_os("ZNIFF_EBPF_OBJ").filter(|s| !s.is_empty()) {
        let src = PathBuf::from(&obj_path);
        println!("cargo:rerun-if-changed={}", src.display());
        fs::copy(&src, &dst).unwrap_or_else(|err| {
            panic!(
                "failed to copy ZNIFF_EBPF_OBJ {} -> {}: {err}",
                src.display(),
                dst.display()
            )
        });
    } else {
        // Placeholder so host tests (`cargo test`) build cleanly even without bpf-linker.
        fs::write(&dst, []).expect("write empty ebpf placeholder");
    }
}
