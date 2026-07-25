#!/usr/bin/env bash
# Build the wasm module and stage the page next to it.
#
#   ./build.sh          # release build into dist/
#   ./build.sh --serve  # ... then serve dist/ on :8000
#
# Requires only the wasm target (`rustup target add wasm32-unknown-unknown`).
# There is no wasm-bindgen, wasm-pack, or npm step: the module is plain
# `extern "C"` Rust and `tuika.js` talks to it directly.
set -euo pipefail

cd "$(dirname "$0")"
out=dist

cargo build --release --target wasm32-unknown-unknown

mkdir -p "$out"
cp target/wasm32-unknown-unknown/release/tuika_web_demo.wasm "$out/"
cp index.html tuika.js "$out/"

# `wasm-opt` (binaryen) typically takes another ~15% off, but it is optional —
# the demo is meant to build with nothing but cargo.
if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -Oz "$out/tuika_web_demo.wasm" -o "$out/tuika_web_demo.wasm"
fi

size=$(wc -c <"$out/tuika_web_demo.wasm")
gz=$(gzip -9 -c "$out/tuika_web_demo.wasm" | wc -c)
printf 'built %s: %s KiB (%s KiB gzipped)\n' \
  "$out/tuika_web_demo.wasm" "$((size / 1024))" "$((gz / 1024))"

if [[ "${1:-}" == "--serve" ]]; then
  echo "serving http://localhost:8000 — ctrl-c to stop"
  python3 -m http.server 8000 --directory "$out"
fi
