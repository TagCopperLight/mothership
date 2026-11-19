#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

lint/comments.py
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
e2e/run.py
