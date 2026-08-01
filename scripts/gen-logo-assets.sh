#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

render_png() {
  local source_svg="$1"
  local output_png="$2"

  if command -v rsvg-convert >/dev/null 2>&1; then
    rsvg-convert --width 1024 --height 1024 "$source_svg" --output "$output_png"
  elif command -v sips >/dev/null 2>&1; then
    sips -s format png "$source_svg" --out "$output_png" >/dev/null
  else
    echo "error: logo PNG export requires rsvg-convert or sips" >&2
    exit 1
  fi
}

render_png "$root_dir/logo.svg" "$root_dir/logo.png"
render_png "$root_dir/logo-dark.svg" "$root_dir/logo-dark.png"
render_png "$root_dir/logo-mono.svg" "$root_dir/logo-mono.png"

echo "wrote logo.png, logo-dark.png, and logo-mono.png"
