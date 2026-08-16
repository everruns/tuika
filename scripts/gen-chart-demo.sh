#!/usr/bin/env bash
# Record every chart kind through both adaptive rendering paths. The portable
# captures use upstream VHS/xterm.js; the graphics captures use the Ghostty VHS
# fork so Kitty images are interpreted and composited into the screenshots.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

for tool in vhs ttyd ffmpeg; do
  command -v "${tool}" >/dev/null || {
    echo "error: ${tool} is required to record the chart demo" >&2
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

echo "Building the charts example…"
cargo build -q -p tuika-charts --example charts
bin="${repo_root}/target/debug/examples/charts"

record_chart() {
  local kind="$1"
  local renderer="$2"
  local recorder="$3"
  local tape="${tapes_dir}/${kind}-${renderer}.tape"
  local output="${output_dir}/${kind}-${renderer}.png"
  local renderer_setting=""
  local environment=""
  local command_env="TERM=xterm-256color TERM_PROGRAM= KITTY_WINDOW_ID= GHOSTTY_RESOURCES_DIR="

  if [[ "${renderer}" == "graphics" ]]; then
    renderer_setting="Set Renderer Ghostty"
    environment="Env TERM_PROGRAM ghostty"
    command_env="TERM=xterm-256color TERM_PROGRAM=ghostty"
  fi

  cat >"${tape}" <<EOF_TAPE
Output "${tapes_dir}/${kind}-${renderer}.gif"

Set Shell bash
${renderer_setting}
Set FontSize 27
Set CursorBlink false
Set Width 1200
Set Height 720
Set Padding 28
Set WindowBar Colorful
Set Theme { "background": "#141214", "foreground": "#ebe6e6" }
${environment}

Hide
Type "clear; ${command_env} TUIKA_CHART_DEMO=${kind} ${bin}"
Show
Enter
Sleep 1.5s
Sleep 300ms
Screenshot "${output}"
Sleep 500ms
Type "q"
Sleep 300ms
EOF_TAPE

  echo "Recording ${kind} (${renderer})…"
  env -u NO_COLOR "${recorder}" "${tape}"
  echo "Wrote ${output} ($(du -h "${output}" | cut -f1))."
}

kinds=("$@")
if [[ ${#kinds[@]} -eq 0 ]]; then
  kinds=(line area bar scatter step grouped stacked horizontal donut focus radar)
fi

for kind in "${kinds[@]}"; do
  record_chart "${kind}" cells vhs
  record_chart "${kind}" graphics "${ghostty_vhs}"
done
