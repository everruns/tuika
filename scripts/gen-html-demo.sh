#!/usr/bin/env bash
#
# Record the tuika-html integration example beside its source.
#
# Requirements: vhs (https://github.com/charmbracelet/vhs), which needs ttyd
# and ffmpeg on PATH. Run from anywhere:
#   scripts/gen-html-demo.sh
#
# The scene is settled — nothing animates — so this captures a full-color PNG
# screenshot rather than a GIF, the same rule the component gallery follows.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

if ! command -v vhs >/dev/null 2>&1; then
  echo "error: vhs not found on PATH (see https://github.com/charmbracelet/vhs)" >&2
  exit 1
fi

echo "Building the HTML Markdown example…"
cargo build -q -p tuika-html --example html_markdown
bin="${repo_root}/target/debug/examples/html_markdown"
output="crates/tuika-html/examples/html_markdown/html.png"

tapes_dir="$(mktemp -d)"
trap 'rm -rf "${tapes_dir}"' EXIT
tape="${tapes_dir}/html.tape"

cat >"${tape}" <<EOF
Output "${tapes_dir}/html.gif"

Set Shell bash
Set FontSize 31
Set CursorBlink false
Set Width 1800
Set Height 1000
Set Padding 36
Set WindowBar Colorful
Set Theme { "background": "#141214", "foreground": "#ebe6e6" }

Hide
Type "clear; TERM=xterm-256color ${bin}"
Enter
Sleep 1.5s
Show
Sleep 300ms
Screenshot ${output}
# The example owns the alternate screen until it is told to quit, so the
# recording ends by quitting it rather than by the process exiting on its own.
Type "q"
Sleep 300ms
EOF

echo "Recording HTML Markdown demo…"
env -u NO_COLOR vhs "${tape}"

echo "Done. Wrote ${output} ($(du -h "${output}" | cut -f1))."
