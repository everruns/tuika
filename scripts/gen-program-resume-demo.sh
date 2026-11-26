#!/usr/bin/env bash
#
# Record the `program_resume` example at examples/program_resume.png.
#
# OSC 7577 is a message to the terminal, and VHS's terminal neither offers a
# token nor restores panes, so this plays the terminal's part the same way
# tests/pty_smoke.rs does: it hands the example a demo token, lets it report
# while chapters change, and then launches the command a restoring terminal
# would run — `program_resume --chapter 4` — which is the scene captured. The
# record panel shows the real command and the signed bytes the binary sent.
#
# The binary runs from a neutral directory so the recording host's paths stay
# out of the image (argv[0] is reported as an absolute path by design).
#
# Requirements: vhs (with ttyd and ffmpeg). Run from anywhere:
#   scripts/gen-program-resume-demo.sh
# Set PROGRAM_RESUME_GIF=path.gif to also keep the whole session as a GIF.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"
source "${repo_root}/scripts/demo-theme.sh"

if ! command -v vhs >/dev/null 2>&1; then
  echo "error: vhs not found on PATH (see https://github.com/charmbracelet/vhs)" >&2
  exit 1
fi

echo "Building the program_resume example…"
cargo build -q --example program_resume
built="${CARGO_TARGET_DIR:-${repo_root}/target}/debug/examples/program_resume"

stage="/tmp/tuika"
mkdir -p "${stage}"
cp "${built}" "${stage}/program_resume"
work_dir="$(mktemp -d)"
trap 'rm -rf "${work_dir}" "${stage}/program_resume"' EXIT
tape="${work_dir}/program-resume.tape"
out="${repo_root}/examples/program_resume.png"
gif="${PROGRAM_RESUME_GIF:-${work_dir}/session.gif}"

# The gallery's density: FontSize 40 at 1760 px wide (2x the 880 px embed).
cat >"${tape}" <<EOF
Output "${gif}"

Set Shell bash
Set FontSize 40
Set CursorBlink false
Set Width 1760
Set Height 1420
Set Padding 22
Set WindowBar Colorful
Set Theme { "background": "${TUIKA_DEMO_BACKGROUND}", "foreground": "${TUIKA_DEMO_FOREGROUND}" }
Set Framerate 24
Set TypingSpeed 50ms

Hide
Type "PS1='\033[38;2;38;139;210m/tmp/tuika\033[0m \$ '"
Enter
Type "cd ${stage} && export OSC7577_TOKEN=demo-token-not-a-secret-0123"
Enter
Type "clear"
Enter
Sleep 500ms
Show

# First life: move to chapter 4; each change is a new signed report.
Type "./program_resume --theme ${TUIKA_DEMO_THEME}"
Enter
Sleep 1500ms
Type "j"
Sleep 700ms
Type "j"
Sleep 700ms
Type "j"
Sleep 2s

# (A real terminal restarts here. VHS cannot, so quit and run exactly what a
# restoring terminal would launch.)
Hide
Type "q"
Sleep 500ms
Type "clear"
Enter
Show
Type "${stage}/program_resume --chapter 4 --theme ${TUIKA_DEMO_THEME}"
Enter
Sleep 2s
Screenshot "${out}"
Sleep 1s
EOF

echo "Recording examples/program_resume.png…"
env -u NO_COLOR vhs "${tape}"

echo "Done. Wrote examples/program_resume.png ($(du -h "${out}" | cut -f1))."
