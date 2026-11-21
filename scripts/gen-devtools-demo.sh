#!/usr/bin/env bash
#
# Record the devtools inspector guide's screenshots from the real `devtools`
# example: the panel after a few routed keys, then the same frame with layout
# outlines switched on.
#
# Requirements: vhs (https://github.com/charmbracelet/vhs), which needs ttyd
# and ffmpeg on PATH. Run from anywhere:
#   scripts/gen-devtools-demo.sh
#
# Both scenes are settled, so they are full-color PNG screenshots rather than
# GIFs, the same rule the component gallery follows.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"
source "${repo_root}/scripts/demo-theme.sh"

if ! command -v vhs >/dev/null 2>&1; then
  echo "error: vhs not found on PATH (see https://github.com/charmbracelet/vhs)" >&2
  exit 1
fi

tapes_dir="$(mktemp -d)"
trap 'rm -rf "${tapes_dir}"' EXIT
output_dir="docs/devtools"
mkdir -p "${output_dir}"

echo "Building the devtools example…"
cargo build -q --example devtools --features devtools

tape="${tapes_dir}/devtools.tape"
cat >"${tape}" <<EOF_TAPE
Output "${tapes_dir}/devtools.gif"

Set Shell bash
Set FontSize 28
Set CursorBlink false
Set Width 1760
Set Height 1080
Set Padding 36
Set WindowBar Colorful
Set Theme { "background": "${TUIKA_DEMO_BACKGROUND}", "foreground": "${TUIKA_DEMO_FOREGROUND}" }

Hide
Type "clear; TERM=xterm-256color ${repo_root}/target/debug/examples/devtools --theme ${TUIKA_DEMO_THEME}"
Enter
Sleep 1.5s
Down
Sleep 200ms
Down
Sleep 200ms
Type "g"
Sleep 200ms
Tab
Sleep 200ms
Type "ship it"
Sleep 300ms
Show
Sleep 300ms
Screenshot ${output_dir}/inspector.png
Hide
Alt+i
Sleep 500ms
Show
Sleep 300ms
Screenshot ${output_dir}/outlines.png
Escape
Sleep 300ms
EOF_TAPE

echo "Recording the devtools inspector…"
env -u NO_COLOR vhs "${tape}"
for shot in inspector outlines; do
  echo "Wrote ${output_dir}/${shot}.png ($(du -h "${output_dir}/${shot}.png" | cut -f1))."
done
