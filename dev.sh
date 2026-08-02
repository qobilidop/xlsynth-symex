#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0

set -euo pipefail

readonly repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly image="${XLSYNTH_SYMEX_DEV_IMAGE:-ghcr.io/qobilidop/xlsynth-symex/dev:main}"

build_image() {
  docker build --file "${repo_dir}/.devcontainer/Dockerfile" \
    --tag "${image}" "${repo_dir}"
}

case "${1:-}" in
  --build) build_image; exit ;;
  --pull) docker pull "${image}"; exit ;;
  --help|-h)
    printf '%s\n' \
      'Usage: ./dev.sh [--build|--pull] [command [args...]]' \
      '' \
      'Runs a command in the development container. With no command, opens a shell.' \
      'Set XLSYNTH_SYMEX_DEV_IMAGE to use a different image or tag.'
    exit
    ;;
esac

if ! docker image inspect "${image}" >/dev/null 2>&1; then
  if ! docker pull "${image}"; then
    printf 'Could not pull %s; building it locally.\n' "${image}" >&2
    build_image
  fi
fi

docker_args=(
  run --rm
  --volume "${repo_dir}:/workspace/xlsynth-symex"
  --workdir /workspace/xlsynth-symex
  --env HOME=/tmp/xlsynth-symex-home
  --env CARGO_HOME=/tmp/xlsynth-symex-home/.cargo
  --env CARGO_TARGET_DIR=/workspace/xlsynth-symex/target
  --user "$(id -u):$(id -g)"
)

if [[ -t 0 && -t 1 ]]; then
  docker_args+=(--interactive --tty)
fi

if (( $# == 0 )); then
  set -- bash
fi

exec docker "${docker_args[@]}" "${image}" "$@"
