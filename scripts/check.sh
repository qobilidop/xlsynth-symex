#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0

set -euo pipefail
set -x

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
