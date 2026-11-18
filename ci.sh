#!/usr/bin/env bash
# The checks CI runs, so a local run matches.
set -euo pipefail
cd "$(dirname "$0")"

cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
e2e/run.py
