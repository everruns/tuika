#!/usr/bin/env bash
#
# Regenerate the stylesheet demo GIFs under docs/styling/styling-*.gif.
#
# Each GIF is the shared scene from examples/styling.rs — one
# markdown block plus panels — painted under a different `tuika::StyleSheet`, so
# the set is an honest side-by-side of what one central styling policy does to
# real components. `styling-cycle.gif` swaps the sheet live so the whole tree
# visibly restyles at once. The variant list is the single source of truth in
# the example's `variants()`; the tapes are generated here, not committed.
#
# Requirements: vhs (https://github.com/charmbracelet/vhs), which needs ttyd and
# ffmpeg on PATH. Run from anywhere:
#   scripts/gen-styling-demos.sh [variant ...]   (no args = all).

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"
source "${repo_root}/scripts/demo-theme.sh"

if ! command -v vhs >/dev/null 2>&1; then
  echo "error: vhs not found on PATH (see https://github.com/charmbracelet/vhs)" >&2
  exit 1
fi

echo "Building the styling example…"
cargo build -q --example styling
bin="${repo_root}/target/debug/examples/styling"
out_dir="${repo_root}/docs/styling"
mkdir -p "${out_dir}"
tapes_dir="$(mktemp -d)"
trap 'rm -rf "${tapes_dir}"' EXIT

bg="$("${bin}" bg --theme "${TUIKA_DEMO_THEME}")"
fg="${TUIKA_DEMO_FOREGROUND}"

# Each held variant plus one live-cycling capture. `run <name>` holds a sheet;
# `run` alone cycles through every variant.
records=("default:run default" "vivid:run vivid" "mono:run mono" "cycle:")

record() {
  local name="$1" cmd="$2" sleep_for="$3"
  local tape
  tape="${tapes_dir}/styling-${name}.tape"
  cat >"${tape}" <<EOF
Output "${out_dir}/styling-${name}.gif"

Set Shell bash
Set FontSize 28
Set CursorBlink false
Set Width 1760
Set Height 1140
Set Padding 34
Set WindowBar Colorful
Set Theme { "background": "${bg}", "foreground": "${fg}" }
Set Framerate 24

Hide
Type "TERM=xterm-256color ${bin} ${cmd} --theme ${TUIKA_DEMO_THEME}"
Enter
Sleep 900ms
Show
Sleep ${sleep_for}
EOF
  echo "Recording styling-${name}.gif…"
  env -u NO_COLOR vhs "${tape}"
}

for entry in "${records[@]}"; do
  name="${entry%%:*}"
  cmd="${entry#*:}"
  if [[ $# -gt 0 ]] && ! printf '%s\n' "$@" | grep -qxF "${name}"; then
    continue
  fi
  # The cycling capture needs long enough to show every sheet; held ones are short.
  if [[ "${name}" == "cycle" ]]; then
    record "${name}" "${cmd}" "9s"
  else
    record "${name}" "${cmd}" "4s"
  fi
done

echo "Done. GIFs written to ${out_dir}/styling-*.gif"
