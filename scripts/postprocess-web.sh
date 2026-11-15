#!/bin/sh
set -eu

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <web_dir> <fonts_dir>" >&2
  exit 1
fi

web_dir="$1"
fonts_dir="$2"

digest10() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print substr($1, 1, 10)}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print substr($1, 1, 10)}'
  else
    echo "error: neither sha256sum nor shasum found" >&2
    exit 1
  fi
}

compress_sidecars() {
  target="$1"
  gzip -9 -c "$target" > "${target}.gz"
  if command -v brotli >/dev/null 2>&1; then
    brotli -f -q 11 -o "${target}.br" "$target"
  else
    rm -f "${target}.br"
  fi
}

mkdir -p "$web_dir/fonts"
rm -f "$web_dir"/fonts/JetBrainsMonoNL-*
for stem in \
  JetBrainsMonoNL-Regular \
  JetBrainsMonoNL-Medium \
  JetBrainsMonoNL-SemiBold \
  JetBrainsMonoNL-Bold
do
  src="${fonts_dir}/${stem}.ttf"
  if [ ! -f "$src" ]; then
    echo "error: missing font $src" >&2
    exit 1
  fi
  digest="$(digest10 "$src")"
  dst="${web_dir}/fonts/${stem}.${digest}.ttf"
  cp "$src" "$dst"
  compress_sidecars "$dst"
done

if command -v wasm-opt >/dev/null 2>&1; then
  wflags="--enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals --enable-reference-types --enable-multivalue --enable-simd --enable-gc --enable-tail-call --enable-extended-const"
  if wasm-opt --help | grep -q -- '--enable-bulk-memory-opt'; then
    wflags="$wflags --enable-bulk-memory-opt"
  fi
  for w in "$web_dir"/nsh.*.wasm; do
    [ -f "$w" ] || continue
    old_base="$(basename "$w")"
    wasm-opt -Oz $wflags "$w" -o "${w}.opt"
    new_digest="$(digest10 "${w}.opt")"
    new_base="nsh.${new_digest}.wasm"
    rm -f "$w" "${w}.gz" "${w}.br"
    mv "${w}.opt" "${web_dir}/${new_base}"
    compress_sidecars "${web_dir}/${new_base}"
    if [ "$old_base" != "$new_base" ] && [ -f "$web_dir/index.html" ]; then
      old_escaped="$(printf '%s' "$old_base" | sed 's/\./\\./g')"
      sed "s|${old_escaped}|${new_base}|g" "$web_dir/index.html" > "${web_dir}/index.html.tmp"
      mv "${web_dir}/index.html.tmp" "$web_dir/index.html"
      grep -q "$new_base" "$web_dir/index.html"
      compress_sidecars "$web_dir/index.html"
    fi
  done
fi
