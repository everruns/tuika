#!/usr/bin/env bash
#
# Regenerate examples/color_scheme.gif — the `color_scheme` example switching
# between its light and dark themes while keys keep arriving.
#
# VHS records through xterm.js, which does not implement mode 2031, so the
# terminal never reports a scheme of its own here; the tape flips it with the
# example's `l` and `d` keys, which set the same `Live` the reports set and take
# the same repaint path. The PTY smoke test covers the report bytes themselves.
#
# The example binary is the source of truth: this drives it under VHS, so the
# recording cannot drift. The tape is generated here, not committed. Being a
# whole-example recording, it sits outside the `demo -- check` invariant.
#
# Requirements: vhs (https://github.com/charmbracelet/vhs), which needs ttyd and
# ffmpeg on PATH. Run from anywhere:
#   scripts/gen-color-scheme-demo.sh

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"
source "${repo_root}/scripts/demo-theme.sh"

if ! command -v vhs >/dev/null 2>&1; then
  echo "error: vhs not found on PATH (see https://github.com/charmbracelet/vhs)" >&2
  exit 1
fi

echo "Building the color_scheme example…"
cargo build -q --example color_scheme
example_dir="${CARGO_TARGET_DIR:-${repo_root}/target}/debug/examples"
example_dir="$(cd "${example_dir}" && pwd)"

tapes_dir="$(mktemp -d)"
trap 'rm -rf "${tapes_dir}"' EXIT
tape="${tapes_dir}/color-scheme.tape"
output="${repo_root}/examples/color_scheme.gif"

# The gallery's pixel density (FontSize 40 at 1760 px, twice the documentation
# embed). Ten rows hold the example's box and its key hints.
cat >"${tape}" <<EOF
Output "${output}"

Set Shell bash
Set FontSize 40
Set CursorBlink false
Set Width 1760
Set Height 680
Set Padding 22
Set WindowBar Colorful
Set Theme { "background": "${TUIKA_DEMO_BACKGROUND}", "foreground": "${TUIKA_DEMO_FOREGROUND}" }
Set Framerate 24
Set TypingSpeed 120ms

Hide
Type "PS1='\$ '"
Enter
Type "cd '${example_dir}' && clear"
Enter
Type "./color_scheme"
Enter
Sleep 1500ms
Show

# Starts on the base theme until a scheme is reported; keys echo meanwhile.
Sleep 1500ms
Type "hi"
Sleep 1s
# Light, then dark, with a key typed in between.
Type "l"
Sleep 2s
Type "x"
Sleep 1s
Type "d"
Sleep 2s
Type "l"
Sleep 2s
Type "q"
Sleep 300ms
EOF

echo "Recording examples/color_scheme.gif…"
before="$(stat -c %Y "${output}" 2>/dev/null || echo 0)"
env -u NO_COLOR vhs "${tape}"
after="$(stat -c %Y "${output}" 2>/dev/null || echo 0)"
if [ "${after}" = "${before}" ]; then
  echo "error: vhs exited without rewriting ${output}" >&2
  exit 1
fi

echo "Done. Wrote examples/color_scheme.gif ($(du -h "${output}" | cut -f1))."
