#!/usr/bin/env bash
#
# Record the deterministic `workbench_demo` example beside its source. The example
# is an in-repo application showcase, so the generated tape stays temporary and
# the checked-in GIF is always captured from the real binary.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

if ! command -v vhs >/dev/null 2>&1; then
  echo "error: vhs not found on PATH (see https://github.com/charmbracelet/vhs)" >&2
  exit 1
fi

echo "Building the workbench_demo example…"
cargo build -q --example workbench_demo
bin="${repo_root}/target/debug/examples/workbench_demo"
out="${repo_root}/examples/workbench_demo/workbench-demo.gif"

tapes_dir="$(mktemp -d)"
trap 'rm -rf "${tapes_dir}"' EXIT
tape="${tapes_dir}/workbench-demo.tape"

# Roughly 96×27 cells, captured above display resolution for a crisp 880 px
# embed. The terminal background matches the example so the padding disappears.
cat >"${tape}" <<EOF
Output "${out}"

Set Shell bash
Set FontSize 28
Set CursorBlink false
Set Width 1800
Set Height 1050
Set Padding 30
Set WindowBar Colorful
Set Theme { "background": "#161215", "foreground": "#deccc7" }
Set Framerate 12

Hide
Type "TERM=xterm-256color ${bin}"
Enter
Sleep 1s
Show
Sleep 2s
Right
Sleep 700ms
Right
Sleep 700ms
Left
Sleep 700ms
Left
Sleep 1500ms
EOF

echo "Recording examples/workbench_demo/workbench-demo.gif…"
env -u NO_COLOR vhs "${tape}"

echo "Done. Wrote ${out} ($(du -h "${out}" | cut -f1))."
