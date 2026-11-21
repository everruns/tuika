#!/usr/bin/env bash
# Record the tuika-charts `latency_dashboard` example through both adaptive
# rendering paths: upstream VHS/xterm.js for portable cells, and the Ghostty VHS
# fork for Kitty graphics (see scripts/gen-chart-demo.sh). The example keeps its
# own palette, so the recording frame is matched to it rather than to the
# repository demo theme.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

for tool in vhs ttyd ffmpeg; do
  command -v "${tool}" >/dev/null || {
    echo "error: ${tool} is required to record the latency dashboard" >&2
    exit 1
  }
done

ghostty_vhs="${VHS_GHOSTTY_BIN:-${repo_root}/target/tools/vhs-ghostty}"
if [[ ! -x "${ghostty_vhs}" ]]; then
  scripts/build-vhs-ghostty.sh
fi

output_dir="${repo_root}/docs/charts"
tapes_dir="$(mktemp -d)"
trap 'rm -rf "${tapes_dir}"' EXIT
mkdir -p "${output_dir}"

echo "Building the latency_dashboard example…"
cargo build -q -p tuika-charts --example latency_dashboard
bin="${repo_root}/target/debug/examples/latency_dashboard"

record() {
  local renderer="$1"
  local recorder="$2"
  local tape="${tapes_dir}/${renderer}.tape"
  local output="${output_dir}/latency-dashboard-${renderer}.png"
  local renderer_setting=""
  local environment=""
  local command_env="TERM=xterm-256color TERM_PROGRAM= KITTY_WINDOW_ID= GHOSTTY_RESOURCES_DIR="

  if [[ "${renderer}" == "graphics" ]]; then
    renderer_setting="Set Renderer Ghostty"
    environment="Env TERM_PROGRAM ghostty"
    command_env="TERM=xterm-256color TERM_PROGRAM=ghostty"
  fi

  cat >"${tape}" <<EOF_TAPE
Output "${tapes_dir}/${renderer}.gif"

Set Shell bash
${renderer_setting}
Set FontSize 24
Set CursorBlink false
Set Width 1760
Set Height 960
Set Padding 28
Set WindowBar Colorful
Set Theme { "background": "#24273a", "foreground": "#cad3f5" }
${environment}

Hide
Type "clear; ${command_env} ${bin}"
Enter
# Wait hidden: two full-plot images are ~2.5 MB of Kitty escapes, and a
# recorder capturing frames meanwhile drains the PTY slowly enough that the
# second image is still arriving at screenshot time.
Sleep 6s
Show
Sleep 500ms
Screenshot "${output}"
Sleep 500ms
Type "q"
Sleep 300ms
EOF_TAPE

  rm -f "${output}"
  echo "Recording latency dashboard (${renderer})…"
  env -u NO_COLOR "${recorder}" "${tape}"
  # vhs can exit 0 without writing; never leave a stale asset behind.
  [[ -s "${output}" ]] || {
    echo "error: ${output} was not written" >&2
    exit 1
  }
  echo "Wrote ${output} ($(du -h "${output}" | cut -f1))."
}

record cells vhs
record graphics "${ghostty_vhs}"
