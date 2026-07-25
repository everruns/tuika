#!/usr/bin/env bash
#
# Regenerate the recording of the `codex` example at examples/codex/codex.gif.
#
# The example is a replica of the Codex CLI's interface built on tuika — not the
# Codex CLI, and unaffiliated with OpenAI. It drives its own scripted agent, so
# the recording is deterministic and offline.
#
# The example itself is the source of truth: this drives the real binary under
# VHS — typing a prompt, opening the slash popup, answering the approval prompt —
# so the recording cannot drift from what the example does. The tape is generated
# here, not committed.
#
# Unlike docs/demos/*.gif, this one is outside the `demo -- check` invariant:
# it records a whole app, not a single component scene, and it lives beside the
# example it records so examples/codex/ stays self-contained.
#
# Requirements: vhs (https://github.com/charmbracelet/vhs), which needs ttyd and
# ffmpeg on PATH. Run from anywhere:
#   scripts/gen-codex-demo.sh

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

if ! command -v vhs >/dev/null 2>&1; then
  echo "error: vhs not found on PATH (see https://github.com/charmbracelet/vhs)" >&2
  exit 1
fi

echo "Building the codex example…"
cargo build -q --example codex
bin="${repo_root}/target/debug/examples/codex"

tapes_dir="$(mktemp -d)"
trap 'rm -rf "${tapes_dir}"' EXIT
tape="${tapes_dir}/codex.tape"

# Sized to ~100×28 cells. The VHS theme background matches the example's own
# palette so the padding blends into the app, and the window bar supplies the
# chrome. Recorded larger than displayed (width="880" in docs/showcases.md) so
# it stays crisp on HiDPI screens.
cat >"${tape}" <<EOF
Output "${repo_root}/examples/codex/codex.gif"

Set Shell bash
Set FontSize 28
Set CursorBlink false
Set Width 1800
Set Height 1132
Set Padding 31
Set WindowBar Colorful
Set Theme { "background": "#0d0e10", "foreground": "#dfe2e6" }
Set Framerate 20
Set TypingSpeed 55ms

Hide
Type "TERM=xterm-256color ${bin}"
Enter
Sleep 1s
Show
Sleep 1500ms

# Slash opens the command palette, filtered as you type…
Type "/"
Sleep 1200ms
Type "mo"
Sleep 1200ms
Backspace 3
Sleep 400ms

# …and at-sign opens a file picker from the same machinery: a different trigger,
# different completions, both colored in the composer as they are typed.
Type "explain @src"
Sleep 1500ms
Tab
Sleep 1200ms
Ctrl+U
Sleep 400ms

# A turn that trips the approval policy.
Type "clean up the build artifacts"
Sleep 500ms
Enter
Sleep 4s
Down
Sleep 700ms
Up
Sleep 500ms
Type "1"
Sleep 7s
EOF

echo "Recording examples/codex/codex.gif…"
env -u NO_COLOR vhs "${tape}"

echo "Done. Wrote ${repo_root}/examples/codex/codex.gif ($(du -h "${repo_root}/examples/codex/codex.gif" | cut -f1))."
