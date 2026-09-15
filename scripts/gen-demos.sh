#!/usr/bin/env bash
#
# Regenerate the tuika component demo assets under docs/demos/.
#
# The scene registry in examples/demo.rs is the single source of
# truth. This script asks the example to emit one VHS tape per scene into a temp
# dir (tapes are generated, not committed), records each, and verifies the
# result. Motion scenes are GIFs; settled scenes are full-color PNG screenshots.
# Both are recorded at exactly 2x pixel density and displayed at half width.
#
# Requirements: vhs (https://github.com/charmbracelet/vhs), which needs ttyd and
# ffmpeg on PATH. Run from anywhere:
#   scripts/gen-demos.sh [scene ...]   (no args = all).

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

if ! command -v vhs >/dev/null 2>&1; then
  echo "error: vhs not found on PATH (see https://github.com/charmbracelet/vhs)" >&2
  exit 1
fi

echo "Building the demo example…"
cargo build -q --example demo

tapes_dir="$(mktemp -d)"
trap 'rm -rf "${tapes_dir}"' EXIT

echo "Emitting tapes…"
cargo run -q --example demo -- tapes "${tapes_dir}"

# The asset a tape writes, read from the tape itself: a motion scene records a
# GIF through `Output`, a settled one a PNG through `Screenshot`. Reading the
# tape keeps this in step with the registry without duplicating its rules.
asset_for() {
  sed -n -E 's#^(Output|Screenshot) (docs/demos/[^"]*)$#\2#p' "$1" | head -1
}

for tape in "${tapes_dir}"/*.tape; do
  name="$(basename "${tape}" .tape)"
  if [[ $# -gt 0 ]] && ! printf '%s\n' "$@" | grep -qxF "${name}"; then
    continue
  fi
  echo "Recording ${name}…"
  asset="$(asset_for "${tape}")"
  marker="${tapes_dir}/${name}.stamp"
  touch "${marker}"
  # Tape Output paths are relative to the repo root.
  # Documentation records the theme's palette even when the caller prefers
  # colorless command output in their own shell.
  (cd "${repo_root}" && env -u NO_COLOR vhs "${tape}")
  # A recorder can fail without failing: vhs 0.12.0 hands ffmpeg an already
  # cancelled context, so it writes nothing, logs nothing, and exits 0 —
  # leaving the previous asset in place to be committed as if it were fresh.
  # Trust the file on disk, not the exit status.
  if [[ ! -f "${asset}" || ! "${asset}" -nt "${marker}" ]]; then
    echo "error: ${asset} was not written by this run; the recorder failed silently" >&2
    echo "hint: vhs 0.12.0 produces no output at all — use a version that does" >&2
    exit 1
  fi
done

echo "Verifying gallery assets…"
cargo run -q --example demo -- check

echo "Done. Assets written to docs/demos/"
