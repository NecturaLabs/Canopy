#!/usr/bin/env bash
# Usage: tools/check.sh
#
# The local part of CI: formatting check, clippy with warnings denied, the full test suite, the
# release tooling's self-test, the licence notices check, the documentation check, the
# vendored-crate check, the hidden-window stall check (Linux, when its tools are installed), the
# installer test (Linux and macOS), and ShellCheck and cargo-deny when they are installed. CI also
# tests on macOS and Windows and checks install.ps1.
# Run it before every commit; it must pass before a change is considered done.
set -euo pipefail

cd "$(dirname "$0")/.."

step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

step "cargo fmt --all --check"
cargo fmt --all --check

step "cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets --locked -- -D warnings

step "cargo test --workspace"
# The tests run git. Give them the identity CI uses and nothing else, so the user's own git
# configuration (signing, hooks paths, aliases, default branch, autocrlf) cannot change results.
test_gitconfig="$(mktemp "${TMPDIR:-/tmp}/canopy-check-gitconfig.XXXXXX")"
trap 'rm -f "$test_gitconfig"' EXIT
git config --file "$test_gitconfig" user.name "Canopy CI"
git config --file "$test_gitconfig" user.email "ci@canopy.invalid"
git config --file "$test_gitconfig" init.defaultBranch main
GIT_CONFIG_GLOBAL="$test_gitconfig" GIT_CONFIG_NOSYSTEM=1 cargo test --workspace --locked

step "tools/release.py selftest"
python3 tools/release.py selftest

step "tools/notices.py --check"
python3 tools/notices.py --check

step "tools/check-docs.py"
python3 tools/check-docs.py

# vendor/eframe must be the published crate plus vendor/eframe.patch (downloads the crate).
step "tools/vendor-eframe.sh --check"
tools/vendor-eframe.sh --check

# Needs a C compiler and libwayland-server; says so and passes when they are missing.
step "tools/stall-check.py"
python3 tools/stall-check.py

case "$(uname -s)" in
  Linux | Darwin)
    step "packaging/test-install.sh"
    sh packaging/test-install.sh >/dev/null
    ;;
  *)
    # install.sh is for Linux and macOS; install.ps1 is checked by CI's Windows job.
    printf '\n(packaging/test-install.sh runs on Linux and macOS only; skipped here)\n'
    ;;
esac

if command -v shellcheck >/dev/null; then
  step "shellcheck"
  shellcheck packaging/*.sh
  shellcheck --severity=warning tools/*.sh
else
  printf '\n(shellcheck not installed; CI runs it)\n'
fi

if command -v cargo-deny >/dev/null; then
  step "cargo deny check"
  cargo deny check
else
  printf '(cargo-deny not installed; CI runs it)\n'
fi

printf '\n\033[1;32mAll checks passed.\033[0m\n'
