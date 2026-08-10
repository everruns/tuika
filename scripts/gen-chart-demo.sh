#!/usr/bin/env bash
# Record the portable cell path of the adaptive chart example. Run the example
# directly in a graphics-capable terminal to exercise the pixel path.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

for tool in vhs ttyd ffmpeg; do
  command -v "${tool}" >/dev/null || {
    echo "error: ${tool} is required to record the chart demo" >&2
    exit 1
  }
done

output="${repo_root}/docs/charts.png"
tapes_dir="$(mktemp -d)"
trap 'rm -rf "${tapes_dir}"' EXIT

echo "Building the charts example…"
cargo build -q -p tuika-charts --example charts
bin="${repo_root}/target/debug/examples/charts"
tape="${tapes_dir}/charts.tape"

cat >"${tape}" <<EOF_TAPE
Output "${tapes_dir}/charts.gif"

Set Shell bash
Set FontSize 31
Set CursorBlink false
Set Width 1800
Set Height 1100
Set Padding 36
Set WindowBar Colorful
Set Theme { "background": "#141214", "foreground": "#ebe6e6" }

Hide
Type "clear; TERM=xterm-256color TERM_PROGRAM= KITTY_WINDOW_ID= GHOSTTY_RESOURCES_DIR= ${bin}"
Show
Enter
Sleep 1.5s
Sleep 300ms
Screenshot "${output}"
Type "q"
Sleep 300ms
EOF_TAPE

echo "Recording portable chart demo…"
env -u NO_COLOR vhs "${tape}"
echo "Wrote ${output} ($(du -h "${output}" | cut -f1))."
