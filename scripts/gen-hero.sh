#!/usr/bin/env bash
#
# Regenerate the README hero GIF at docs/hero.gif.
#
# The composite gallery scene in examples/screenshot.rs is the
# single source of truth (the same `scene()` also renders docs/hero.svg). This
# script builds that example, records its `run` mode under VHS, and writes the
# GIF. The tape is generated here, not committed.
#
# Requirements: vhs (https://github.com/charmbracelet/vhs), which needs ttyd and
# ffmpeg on PATH. Run from anywhere:
#   scripts/gen-hero.sh

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

if ! command -v vhs >/dev/null 2>&1; then
  echo "error: vhs not found on PATH (see https://github.com/charmbracelet/vhs)" >&2
  exit 1
fi

echo "Building the screenshot example…"
cargo build -q --example screenshot
bin="${repo_root}/target/debug/examples/screenshot"

tape="$(mktemp --suffix=.tape)"
trap 'rm -f "${tape}"' EXIT

# The scene fills the terminal, so the window is sized to yield ~92×30 cells.
# The theme background matches tuika's own so VHS's padding blends into the app,
# and the colorful window bar mirrors the chrome the SVG draws itself. Recorded
# larger than displayed (width="880" in the README) so it stays crisp on HiDPI.
cat >"${tape}" <<EOF
Output "${repo_root}/docs/hero.gif"

Set Shell bash
Set FontSize 26
Set Width 1500
Set Height 1040
Set Padding 30
Set WindowBar Colorful
Set Theme { "background": "#141214", "foreground": "#ebe6e6" }
Set Framerate 24

Hide
Type "TERM=xterm-256color ${bin} run"
Enter
Sleep 900ms
Show
Sleep 6s
EOF

echo "Recording docs/hero.gif…"
vhs "${tape}"

echo "Done. Wrote ${repo_root}/docs/hero.gif ($(du -h "${repo_root}/docs/hero.gif" | cut -f1))."
