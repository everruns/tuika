#!/usr/bin/env bash
#
# Regenerate the tuika-codeformatters language gallery recording.
#
# Requirements: vhs (https://github.com/charmbracelet/vhs), which needs ttyd
# and ffmpeg on PATH. Run from anywhere:
#   scripts/gen-language-demo.sh

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"
source "${repo_root}/scripts/demo-theme.sh"

if ! command -v vhs >/dev/null 2>&1; then
  echo "error: vhs not found on PATH (see https://github.com/charmbracelet/vhs)" >&2
  exit 1
fi

echo "Building the language gallery example…"
cargo build -q -p tuika-codeformatters --example languages
bin="${repo_root}/target/debug/examples/languages"
output="${repo_root}/crates/tuika-codeformatters/docs/languages.gif"

tapes_dir="$(mktemp -d)"
trap 'rm -rf "${tapes_dir}"' EXIT
tape="${tapes_dir}/languages.tape"

cat >"${tape}" <<EOF
Output "${output}"

Set Shell bash
Set FontSize 30
Set CursorBlink false
Set Width 1760
Set Height 1000
Set Padding 32
Set WindowBar Colorful
Set Theme { "background": "${TUIKA_DEMO_BACKGROUND}", "foreground": "${TUIKA_DEMO_FOREGROUND}" }
Set Framerate 12

Hide
Type "clear; TERM=xterm-256color ${bin} --theme ${TUIKA_DEMO_THEME}"
Enter
Sleep 900ms
Show
Sleep 1200ms
Right
Sleep 900ms
Right
Sleep 900ms
Right
Sleep 900ms
Right
Sleep 900ms
Right
Sleep 1200ms
EOF

echo "Recording language gallery demo…"
env -u NO_COLOR vhs "${tape}"

echo "Done. Wrote ${output} ($(du -h "${output}" | cut -f1))."
