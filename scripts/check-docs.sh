#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0

set -euo pipefail

RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
