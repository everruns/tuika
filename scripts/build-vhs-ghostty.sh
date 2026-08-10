#!/usr/bin/env bash
# Build the Ghostty-enabled VHS fork used for recordings that contain terminal
# graphics. The checkout and binary stay under target/ and are never committed.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_dir="${repo_root}/target/vendor/vhs"
output="${repo_root}/target/tools/vhs-ghostty"
revision="f3c8506387f45188334b66470ad74fc04d794fd1"

for tool in git go pkg-config zig; do
  command -v "${tool}" >/dev/null || {
    echo "error: ${tool} is required to build the Ghostty VHS renderer" >&2
    exit 1
  }
done

mkdir -p "${repo_root}/target/vendor" "${repo_root}/target/tools"
if [[ ! -d "${source_dir}/.git" ]]; then
  git clone --filter=blob:none https://github.com/everruns/vhs.git "${source_dir}"
fi

git -C "${source_dir}" fetch --depth 1 origin "${revision}"
git -C "${source_dir}" checkout --detach "${revision}"

patch="${repo_root}/scripts/patches/vhs-ghostty-images.patch"
if ! git -C "${source_dir}" apply --reverse --check "${patch}" 2>/dev/null; then
  git -C "${source_dir}" apply "${patch}"
fi

export PKG_CONFIG_PATH="$(cd "${source_dir}" && scripts/build-libghostty-vt.sh)"
(cd "${source_dir}" && go build -tags ghostty -o "${output}" .)

echo "Built ${output}"
