#!/usr/bin/env bash
#
# Record the tuika-mermaid integration example beside its source.
#
# Requirements: vhs (https://github.com/charmbracelet/vhs), which needs ttyd
# and ffmpeg on PATH. Run from anywhere:
#   scripts/gen-mermaid-demo.sh

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

if ! command -v vhs >/dev/null 2>&1; then
  echo "error: vhs not found on PATH (see https://github.com/charmbracelet/vhs)" >&2
  exit 1
fi

echo "Building the Mermaid Markdown example…"
cargo build -q -p tuika-mermaid --example mermaid_markdown
bin="${repo_root}/target/debug/examples/mermaid_markdown"
output="${repo_root}/crates/tuika-mermaid/examples/mermaid_markdown/mermaid.gif"

tapes_dir="$(mktemp -d)"
trap 'rm -rf "${tapes_dir}"' EXIT
tape="${tapes_dir}/mermaid.tape"

cat >"${tape}" <<EOF
Output "${output}"

Set Shell bash
Set FontSize 26
Set Width 1500
Set Height 620
Set Padding 30
Set WindowBar Colorful
Set Theme { "background": "#141214", "foreground": "#ebe6e6" }
Set Framerate 12

Hide
Type "clear; TERM=xterm-256color ${bin}"
Enter
Sleep 500ms
Show
Sleep 3s
EOF

echo "Recording Mermaid Markdown demo…"
env -u NO_COLOR vhs "${tape}"

echo "Done. Wrote ${output} ($(du -h "${output}" | cut -f1))."
