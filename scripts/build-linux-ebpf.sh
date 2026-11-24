#!/usr/bin/env sh
# Build the Linux eBPF transparent proxy helper (`zniff-ebpf-helper`) with
# embedded `zniff-ebpf-prog` ELF bytecode.
# Usage: scripts/build-linux-ebpf.sh [dest_binary]
set -eu

if [ "$(uname -s)" != "Linux" ]; then
  echo "build-linux-ebpf.sh only runs on Linux" >&2
  exit 1
fi

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
dest=${1:-"$root/linux/ebpf/target/release/zniff-ebpf-helper"}

if ! command -v bpf-linker >/dev/null 2>&1; then
  echo "missing bpf-linker in PATH (install via: cargo install --locked bpf-linker)" >&2
  exit 1
fi

cargo +nightly build \
  --manifest-path "$root/linux/ebpf/zniff-ebpf-prog/Cargo.toml" \
  --target bpfel-unknown-none \
  -Z build-std=core \
  --release

ebpf_obj="$root/linux/ebpf/zniff-ebpf-prog/target/bpfel-unknown-none/release/zniff-ebpf-prog"
if [ ! -f "$ebpf_obj" ]; then
  echo "missing built eBPF ELF: $ebpf_obj" >&2
  exit 1
fi

ZNIFF_EBPF_OBJ="$ebpf_obj" \
  cargo build \
    --manifest-path "$root/linux/ebpf/Cargo.toml" \
    --locked \
    -p zniff-ebpf-helper \
    --release

built="$root/linux/ebpf/target/release/zniff-ebpf-helper"
if [ ! -f "$built" ]; then
  echo "missing built helper binary: $built" >&2
  exit 1
fi

if [ "$built" != "$dest" ]; then
  mkdir -p "$(dirname "$dest")"
  cp "$built" "$dest"
fi
chmod 755 "$dest"
echo "$dest"
