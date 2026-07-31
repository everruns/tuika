#!/usr/bin/env bash
#
# Record the tuika-html examples beside their sources: the markdown seam
# (`html_markdown`) and the standalone `Html` component (`html_view`).
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

tapes_dir="$(mktemp -d)"
trap 'rm -rf "${tapes_dir}"' EXIT

# Both examples own the alternate screen until told to quit, so each tape ends
# by quitting rather than by the process exiting on its own.
record() {
  local name="$1" height="$2" output="$3"
  echo "Building the ${name} example…"
  cargo build -q -p tuika-html --example "${name}"
  local tape="${tapes_dir}/${name}.tape"
  cat >"${tape}" <<EOF
Output "${tapes_dir}/${name}.gif"

Set Shell bash
Set FontSize 31
Set CursorBlink false
Set Width 1800
Set Height ${height}
Set Padding 36
Set WindowBar Colorful
Set Theme { "background": "#141214", "foreground": "#ebe6e6" }

Hide
Type "clear; TERM=xterm-256color ${repo_root}/target/debug/examples/${name}"
Enter
Sleep 1.5s
Show
Sleep 300ms
Screenshot ${output}
Type "q"
Sleep 300ms
EOF
  echo "Recording ${name} demo…"
  env -u NO_COLOR vhs "${tape}"
  echo "Wrote ${output} ($(du -h "${output}" | cut -f1))."
}

# The seam: HTML blocks inside a markdown document.
record html_markdown 1000 "crates/tuika-html/examples/html_markdown/html.png"
# The component: a whole HTML fragment as the screen.
record html_view 1560 "crates/tuika-html/examples/html_view/html_view.png"

echo "Done."
