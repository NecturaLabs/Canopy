#!/usr/bin/env bash
# Usage: tools/check.sh
#
# The local equivalent of CI: formatting check, clippy with warnings denied, the full test suite,
# the release tooling's self-test and the licence notices check (cargo-deny runs only in CI).
# Run it before every commit; it must pass before a change is considered done.
set -euo pipefail

cd "$(dirname "$0")/.."

step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

step "cargo fmt --all --check"
cargo fmt --all --check

step "cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets --locked -- -D warnings

step "cargo test --workspace"
cargo test --workspace --locked

step "tools/release.py selftest"
python3 tools/release.py selftest

step "tools/notices.py --check"
python3 tools/notices.py --check

printf '\n\033[1;32mAll checks passed.\033[0m\n'
