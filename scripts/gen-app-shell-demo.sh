#!/usr/bin/env bash
#
# Record the interactive `app_shell` example beside its source. The generated
# tape stays temporary; the checked-in GIF is always captured from the real
# binary so navigation and status updates cannot drift from the example.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"
source "${repo_root}/scripts/demo-theme.sh"

if ! command -v vhs >/dev/null 2>&1; then
  echo "error: vhs not found on PATH (see https://github.com/charmbracelet/vhs)" >&2
  exit 1
fi

echo "Building the app_shell example…"
cargo build -q --example app_shell
bin="${repo_root}/target/debug/examples/app_shell"
out="${repo_root}/examples/app_shell.gif"

tapes_dir="$(mktemp -d)"
trap 'rm -rf "${tapes_dir}"' EXIT
tape="${tapes_dir}/app-shell.tape"

# 66 columns at the component gallery's pixel density. The height leaves enough
# room for the shell's complete chrome and growing body, while the exact 1760 px
# width stays at 2× its 880 px documentation embed.
cat >"${tape}" <<EOF
Output "${out}"

Set Shell bash
Set FontSize 40
Set CursorBlink false
Set Width 1760
Set Height 850
Set Padding 22
Set WindowBar Colorful
Set Theme { "background": "${TUIKA_DEMO_BACKGROUND}", "foreground": "${TUIKA_DEMO_FOREGROUND}" }
Set Framerate 20

Hide
Type "TERM=xterm-256color ${bin} --theme ${TUIKA_DEMO_THEME}"
Enter
Sleep 900ms
Show
Sleep 1200ms
Down
Sleep 700ms
Down
Sleep 700ms
Enter
Sleep 1200ms
Up
Sleep 1200ms
EOF

echo "Recording examples/app_shell.gif…"
env -u NO_COLOR vhs "${tape}"

dimensions="$(ffprobe -v error -select_streams v:0 \
  -show_entries stream=width,height -of csv=s=x:p=0 "${out}")"
if [[ "${dimensions}" != "1760x850" ]]; then
  echo "error: AppShell recording is ${dimensions}; expected 1760x850" >&2
  exit 1
fi

duration="$(ffprobe -v error -show_entries format=duration -of csv=p=0 "${out}")"
if ! awk -v duration="${duration}" 'BEGIN { exit !(duration >= 4.9 && duration <= 5.5) }'; then
  echo "error: AppShell recording is ${duration}s; expected about 5s" >&2
  exit 1
fi

echo "Done. Wrote ${out} ($(du -h "${out}" | cut -f1))."
