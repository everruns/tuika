#!/usr/bin/env bash
#
# Regenerate every committed demo asset in the workspace.
#
# Showcase recordings clone/build external applications and are included by
# default. Pass --skip-showcases for an otherwise complete local-only refresh.
#
# Requirements are the union of the individual generators documented in
# AGENTS.md: Cargo, VHS/ttyd/ffmpeg, and the showcase tools when included.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${repo_root}"

skip_showcases=false
if [[ ${1-} == "--skip-showcases" ]]; then
  skip_showcases=true
  shift
fi
if [[ $# -ne 0 ]]; then
  echo "usage: scripts/gen-all-demos.sh [--skip-showcases]" >&2
  exit 2
fi

scripts/gen-demos.sh
scripts/gen-hero.sh
scripts/gen-theme-demos.sh
scripts/gen-styling-demos.sh

scripts/gen-split-footer-demo.sh

echo "Generating image SVG…"
cargo run -q --example image_demo -- docs/demos/image.svg


scripts/gen-codex-demo.sh
scripts/gen-mermaid-demo.sh
scripts/gen-html-demo.sh
scripts/gen-language-demo.sh
scripts/gen-chart-demo.sh

if [[ "${skip_showcases}" == false ]]; then
  scripts/gen-showcase-demos.sh
fi

echo "Verifying component asset inventory…"
cargo run -q --example demo -- check

echo "Done. Regenerated every committed demo asset."
