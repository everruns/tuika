#!/usr/bin/env bash
#
# Regenerate docs/split-footer.gif — the recording for the split-footer
# screen mode, embedded in the README, docs/features.md, and `ScreenMode`'s
# rustdoc.
#
# What the picture has to show is a *whole terminal*, not a frame: the footer
# pinned to the last rows, the published output above it as ordinary scrollback,
# growing and shrinking without losing output — and, after `q`, the same scrollback
# with the footer's rows handed back. None of that lives in a `Buffer`, so the
# scene cannot come from the `demo` registry; it is recorded from the real
# session, in a real terminal, like the Codex example.
#
# The `split_footer` example is the source of truth: this drives its own binary
# under VHS, so the recording cannot drift from what the example does. The tape
# is generated here, not committed. Being a whole-terminal recording rather than
# a single-component scene, it sits outside the `demo -- check` invariant.
#
# Requirements: vhs (https://github.com/charmbracelet/vhs), which needs ttyd and
# ffmpeg on PATH. Run from anywhere:
#   scripts/gen-split-footer-demo.sh

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"
source "${repo_root}/scripts/demo-theme.sh"

if ! command -v vhs >/dev/null 2>&1; then
  echo "error: vhs not found on PATH (see https://github.com/charmbracelet/vhs)" >&2
  exit 1
fi

echo "Building the split_footer example…"
# Launch the built binary directly: unrelated Cargo builds must not block the
# recording while the tape is already sending input.
cargo build -q --example split_footer
example_dir="${CARGO_TARGET_DIR:-${repo_root}/target}/debug/examples"
example_dir="$(cd "${example_dir}" && pwd)"

tapes_dir="$(mktemp -d)"
trap 'rm -rf "${tapes_dir}"' EXIT
tape="${tapes_dir}/split-footer.tape"

# 66×14 cells at the component gallery's pixel density (FontSize 40, ~26×55 px
# cells), so this sits beside the demos without looking softer. The 1760 px
# width is exactly twice the documentation embed, avoiding fractional browser
# resampling. Fourteen rows is the smallest grid that holds the whole story at
# once: the command line, the
# blocks the worker publishes, and the five-row footer under them. Solarized Dark
# is the palette every repository-owned documentation capture uses.
cat >"${tape}" <<EOF
Output "${repo_root}/docs/split-footer.gif"

Set Shell bash
Set FontSize 40
Set CursorBlink false
Set Width 1760
Set Height 850
Set Padding 22
Set WindowBar Colorful
Set Theme { "background": "${TUIKA_DEMO_BACKGROUND}", "foreground": "${TUIKA_DEMO_FOREGROUND}" }
Set Framerate 24
Set TypingSpeed 60ms

# A prompt of our own: the default one carries the recording host's user and
# path, and its color would come from outside the theme.
Hide
Type "PS1='\033[38;2;38;139;210m~/src/tuika\033[0m \$ '"
Enter
Type "cd '${example_dir}'"
Enter
Type "clear"
Enter
Sleep 500ms
Show

# The actual example binary, with no build output in the recorded scrollback.
Type "./split_footer --theme ${TUIKA_DEMO_THEME}"
Sleep 400ms
Enter

# Publish, grow from five to seven rows, then return to five while the worker
# keeps publishing. The last rows move; existing output stays in scrollback.
Sleep 2s
Type "+"
Sleep 1s
Type "+"
Sleep 2s
Type "-"
Sleep 1s
Type "-"
Sleep 2s

# The point of the mode: quitting releases the footer's rows and leaves every
# published block behind as the terminal's own scrollback.
Type "q"
Sleep 3s
EOF

echo "Recording docs/split-footer.gif…"
env -u NO_COLOR vhs "${tape}"

echo "Done. Wrote docs/split-footer.gif ($(du -h docs/split-footer.gif | cut -f1))."
