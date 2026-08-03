#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0

set -euo pipefail

check_local_links() {
  local failed=0
  local document
  local target
  local resolved

  while IFS= read -r document; do
    while IFS= read -r target; do
      case "${target}" in
        http://*|https://*|mailto:*|\#*) continue ;;
      esac
      target="${target%%#*}"
      [[ -z "${target}" ]] && continue
      resolved="$(dirname "${document}")/${target}"
      if [[ ! -e "${resolved}" ]]; then
        printf '%s: broken local link: %s\n' "${document}" "${target}" >&2
        failed=1
      fi
    done < <(
      grep -oE '\]\([^)]*\)' "${document}" \
        | sed -e 's/^](//' -e 's/)$//'
    )
  done < <(find README.md docs -type f -name '*.md' -print)

  return "${failed}"
}

check_local_links

cargo clean --doc
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
mdbook clean
mdbook build

mkdir -p target/site/api
cp -R target/doc/. target/site/api/

test -f target/site/index.html
test -f target/site/404.html
test -f target/site/user/support-matrix.html
test -f target/site/api/xlsynth_symex/index.html
