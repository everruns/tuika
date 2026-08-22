#!/usr/bin/env bash
#
# Regenerate the README hero GIF at docs/hero.gif.
#
# The composite gallery scene in examples/screenshot.rs is the
# single source of truth (the same `scene()` also renders docs/hero.svg). This
# script builds that example, records its default terminal mode under VHS, and
# writes the GIF. The tape is generated here, not committed.
#
# Requirements: vhs (https://github.com/charmbracelet/vhs), which needs ttyd and
# ffmpeg on PATH. Run from anywhere:
#   scripts/gen-hero.sh

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"
source "${repo_root}/scripts/demo-theme.sh"

if ! command -v vhs >/dev/null 2>&1; then
  echo "error: vhs not found on PATH (see https://github.com/charmbracelet/vhs)" >&2
  exit 1
fi

echo "Building the screenshot example…"
cargo build -q --example screenshot
bin="${repo_root}/target/debug/examples/screenshot"

tapes_dir="$(mktemp -d)"
trap 'rm -rf "${tapes_dir}"' EXIT
tape="${tapes_dir}/hero.tape"

# The scene fills the terminal, so the window is sized to yield ~100×34 cells.
# The theme background matches tuika's own so VHS's padding blends into the app,
# and the colorful window bar mirrors the chrome the SVG draws itself. The
# 1760 px recording is exactly twice its 880 px README display width.
cat >"${tape}" <<EOF
Output "${repo_root}/docs/hero.gif"

Set Shell bash
Set FontSize 28
Set CursorBlink false
Set Width 1760
Set Height 1248
Set Padding 36
Set WindowBar Colorful
Set Theme { "background": "${TUIKA_DEMO_BACKGROUND}", "foreground": "${TUIKA_DEMO_FOREGROUND}" }
Set Framerate 24

Hide
Type "TERM=xterm-256color ${bin} --theme ${TUIKA_DEMO_THEME}"
Enter
Sleep 900ms
Show
Sleep 6s
EOF

echo "Recording docs/hero.gif…"
env -u NO_COLOR vhs "${tape}"

echo "Done. Wrote ${repo_root}/docs/hero.gif ($(du -h "${repo_root}/docs/hero.gif" | cut -f1))."
