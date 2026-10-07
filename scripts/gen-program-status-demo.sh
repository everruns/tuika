#!/usr/bin/env bash
#
# Record the `program_status` example inside tuios at
# examples/program_status.gif.
#
# OSC 7501 is a message to the terminal, so a recording of the app alone would
# show nothing of it. This runs the real example in a tuios pane and, beside it,
# a pane that polls `tuios get-agent-state` for that pane, so the recording shows
# the reports the app sends (its lower panel) and the record tuios keeps from
# them (the right pane, and the agents rail) changing together: working with
# rising progress, blocked on a question, done after `y`, and gone after `q`.
#
# Like the codex and app_shell recordings it records a whole app, so it is
# outside the `demo -- check` invariant and lives beside the example.
#
# Requirements: vhs (with ttyd and ffmpeg), jq, and tuios with OSC 7501 support
# (`tuios status` exists; built from https://github.com/Gaurav-Gosain/tuios
# main until a release carries it). Run from anywhere:
#   scripts/gen-program-status-demo.sh

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"
source "${repo_root}/scripts/demo-theme.sh"

for tool in vhs jq tuios; do
  if ! command -v "${tool}" >/dev/null 2>&1; then
    echo "error: ${tool} not found on PATH" >&2
    exit 1
  fi
done
if ! tuios status --help >/dev/null 2>&1; then
  echo "error: this tuios has no OSC 7501 support (no 'tuios status')" >&2
  exit 1
fi

echo "Building the program_status example…"
cargo build -q --example program_status
bin="${repo_root}/target/debug/examples/program_status"
out="${repo_root}/examples/program_status.gif"

work_dir="$(mktemp -d)"
# An empty directory for the session, so tuios's file sidebar stays quiet.
mkdir "${work_dir}/session"
session="tuika-program-status-$$"
cleanup() {
  tuios kill-session "${session}" >/dev/null 2>&1 || true
  rm -rf "${work_dir}"
}
trap cleanup EXIT

# The right-hand pane: what tuios itself recorded for the `build` pane.
watch="${work_dir}/watch.sh"
cat >"${watch}" <<'EOF'
#!/bin/sh
while :; do
  out=$(tuios get-agent-state -w build --json 2>/dev/null | jq -C 'select(.success) | {state, blocked_by, program_status: [.program_status[]? | {state, app, progress, kind, msg} | with_entries(select(.value != null and .value != -1))]}')
  [ -n "$out" ] || out="(no pane named build: no record)"
  printf '\033[H\033[2J\033[1mtuios get-agent-state -w build\033[0m\n\n%s\n' "$out"
  sleep 0.3
done
EOF
chmod +x "${watch}"

# The app runs with its default theme, which inherits the pane's colors, and
# tuios draws the panes in Solarized Dark, matching the documentation palette.
# Two panes and the tuios sidebar need ~170 columns, so this runs at a smaller
# font than the component gallery; the 1760 px width still matches the 2×
# documentation embed. The job reaches its question at ~7s and finishes ~1s
# after the `y`.
tape="${work_dir}/program-status.tape"
cat >"${tape}" <<EOF
Output "${out}"

Set Shell bash
Set FontSize 16
Set CursorBlink false
Set Width 1760
Set Height 990
Set Padding 16
Set WindowBar Colorful
Set Theme { "background": "${TUIKA_DEMO_BACKGROUND}", "foreground": "${TUIKA_DEMO_FOREGROUND}" }
Set Framerate 20

Hide
Type "export TERM=xterm-256color; tuios new -d ${session} --cwd '${work_dir}/session' >/dev/null && sleep 0.5 && tuios new-window -s ${session} build '${bin}' >/dev/null && tuios new-window -s ${session} --no-focus tuios '${watch}' >/dev/null && tuios close-window -s ${session} 0 >/dev/null && tuios attach ${session} --ascii-only --terminal-mode --theme builtin_solarized_dark"
Enter
Sleep 2500ms
Show
Sleep 9s
Type "y"
Sleep 4s
Type "q"
Sleep 2500ms
EOF

echo "Recording examples/program_status.gif…"
marker="${work_dir}/before-recording"
touch "${marker}"
env -u NO_COLOR vhs "${tape}"

# vhs can exit 0 without writing; a stale asset must not pass as fresh.
if [[ ! "${out}" -nt "${marker}" ]]; then
  echo "error: vhs did not rewrite ${out}" >&2
  exit 1
fi

dimensions="$(ffprobe -v error -select_streams v:0 \
  -show_entries stream=width,height -of csv=s=x:p=0 "${out}")"
if [[ "${dimensions}" != "1760x990" ]]; then
  echo "error: program_status recording is ${dimensions}; expected 1760x990" >&2
  exit 1
fi

echo "Done. Wrote ${out} ($(du -h "${out}" | cut -f1))."
