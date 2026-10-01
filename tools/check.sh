#!/usr/bin/env bash
# Usage: tools/check.sh
#
# The local equivalent of CI: formatting check, clippy with warnings denied, and the full test
# suite. Run it before every commit; it must pass before a change is considered done.
set -euo pipefail

cd "$(dirname "$0")/.."

step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

step "cargo fmt --all --check"
cargo fmt --all --check

step "cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets --locked -- -D warnings

step "cargo test --workspace"
cargo test --workspace --locked

printf '\n\033[1;32mAll checks passed.\033[0m\n'
