#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0

set -euo pipefail

readonly repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly image="${XLSYNTH_SYMEX_DEV_IMAGE:-ghcr.io/qobilidop/xlsynth-symex/dev:main}"

if (( $# == 0 )); then
  printf 'Usage: ./dev.sh command [args...]\n' >&2
  exit 2
fi

mkdir -p "${repo_dir}/.cache/tmp"

terminal_args=(--interactive)
if [[ -t 0 && -t 1 ]]; then
  terminal_args+=(--tty)
fi

exec docker run --rm "${terminal_args[@]}" \
  --platform linux/amd64 \
  --volume "${repo_dir}:/workspace/xlsynth-symex" \
  --volume "${repo_dir}/.cache/tmp:/tmp" \
  --workdir /workspace/xlsynth-symex \
  --env HOME=/tmp \
  --env CARGO_HOME=/workspace/xlsynth-symex/.cache/cargo \
  --env CARGO_TARGET_DIR=/workspace/xlsynth-symex/target \
  --user "$(id -u):$(id -g)" \
  "${image}" "$@"
