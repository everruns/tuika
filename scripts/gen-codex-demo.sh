#!/usr/bin/env bash
#
# Regenerate the recordings of the `codex` example, beside the example itself:
# examples/codex/codex.gif (full screen) and codex-split-footer.gif (the same app
# over live terminal scrollback).
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
#   scripts/gen-codex-demo.sh                 # both
#   scripts/gen-codex-demo.sh split-footer    # just one

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

scenes=("${@}")
if [[ ${#scenes[@]} -eq 0 ]]; then
  scenes=(full-screen split-footer)
fi

record_full_screen() {

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
  echo "Wrote examples/codex/codex.gif ($(du -h "${repo_root}/examples/codex/codex.gif" | cut -f1))."
}

# The split-footer scene records the *whole terminal*, not just the app: the
# session scrolls into the terminal's own scrollback as each entry settles, the
# composer keeps the bottom rows and resizes to what it needs, and the last beat
# is the app exiting — the transcript stays, the reserved rows go back.
record_split_footer() {
  # Same capture geometry as the full-screen scene, so the two sit side by side
  # in docs/showcases.md at the same sharpness — but at a lower framerate. This
  # scene repaints the *whole* terminal as published blocks scroll the session
  # up, and at 20 fps the capture cannot keep up: it drops frames, and the GIF
  # then plays back a third faster than the session really ran. Check the
  # duration against the tape after changing anything here.
  cat >"${tape}" <<EOF
Output "${repo_root}/examples/codex/codex-split-footer.gif"

Set Shell bash
Set FontSize 28
Set CursorBlink false
Set Width 1800
Set Height 1132
Set Padding 31
Set WindowBar Colorful
Set Theme { "background": "#0d0e10", "foreground": "#dfe2e6" }
Set Framerate 12
Set TypingSpeed 55ms

Hide
Type "clear"
Enter
Sleep 500ms
Show
Type "codex --split-footer"
Sleep 600ms
Hide
Ctrl+U
Type "TERM=xterm-256color ${bin} --split-footer"
Enter
Sleep 1s
Show
Sleep 1200ms

# The popup takes rows from the scrollback and hands them straight back. Escape
# dismisses the popup but keeps what was typed, so clear the composer too.
Type "/"
Sleep 1400ms
Escape
Sleep 700ms
Ctrl+U
Sleep 400ms

# A turn: each finished entry is published into the terminal above the composer.
Type "explain this repo"
Sleep 400ms
Enter
Sleep 9s

# Quitting releases the reserved rows — the session stays where it was printed.
Ctrl+C
Sleep 800ms
Type "echo 'the transcript above is ordinary scrollback'"
Enter
Sleep 2s
EOF

  echo "Recording examples/codex/codex-split-footer.gif…"
  env -u NO_COLOR vhs "${tape}"
  echo "Wrote examples/codex/codex-split-footer.gif ($(du -h "${repo_root}/examples/codex/codex-split-footer.gif" | cut -f1))."
}

for scene in "${scenes[@]}"; do
  case "${scene}" in
    full-screen) record_full_screen ;;
    split-footer) record_split_footer ;;
    *)
      echo "error: unknown scene '${scene}' (full-screen | split-footer)" >&2
      exit 1
      ;;
  esac
done
