#!/usr/bin/env bash
# Usage: tools/check.sh
#
# The local part of CI: formatting check, clippy with warnings denied, the unsafe-code policy
# (only crates/canopy-app/src/ui/kit/glass/gl.rs may contain unsafe code), the full test suite, the
# release tooling's self-test, the licence notices check, the documentation check, the site
# check, the vendored-crate check, the hidden-window stall check (Linux, when its tools are
# installed), the installer test (Linux and macOS), and ShellCheck and cargo-deny when they are
# installed. CI also tests on macOS and Windows; nothing in CI runs install.ps1.
# Run it before every commit; it must pass before a change is considered done.
set -euo pipefail

cd "$(dirname "$0")/.."

step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

step "cargo fmt --all --check"
cargo fmt --all --check

step "cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets --locked -- -D warnings

# Unsafe code only in the backdrop blur's OpenGL module: clippy denies it elsewhere, and this
# also fails on unsafe code or an allow for it in any other file (comments and strings aside), on
# `include!` or a `#[path]` that hides code from that scan, and on unreadable directories.
step "unsafe policy (only crates/canopy-app/src/ui/kit/glass/gl.rs)"
cargo test -p canopy-app --locked --test unsafe_policy

step "Rust source header guard"
python3 tools/check-source-headers.py --selftest
python3 tools/check-source-headers.py
python3 tools/check-numeric-literals.py --selftest
python3 tools/check-numeric-literals.py

step "UI colour architecture guard"
python3 tools/check-ui-colors.py --selftest
python3 tools/check-ui-colors.py

step "cargo test --workspace"
# The tests run git. Give them the identity CI uses and nothing else, so the user's own git
# configuration (signing, hooks paths, aliases, default branch, autocrlf) cannot change results.
test_gitconfig="$(mktemp "${TMPDIR:-/tmp}/canopy-check-gitconfig.XXXXXX")"
trap 'rm -f "$test_gitconfig"' EXIT
git config --file "$test_gitconfig" user.name "Canopy CI"
git config --file "$test_gitconfig" user.email "ci@canopy.invalid"
git config --file "$test_gitconfig" init.defaultBranch main
# Every test binary runs even after one fails, so one run reports every failure, as in CI.
GIT_CONFIG_GLOBAL="$test_gitconfig" GIT_CONFIG_NOSYSTEM=1 cargo test --workspace --locked --no-fail-fast

step "tools/release.py selftest"
python3 tools/release.py selftest

step "pre-commit hook regressions"
python3 tools/test-hooks.py

step "tools/notices.py --check"
python3 tools/notices.py --check

step "tools/check-docs.py"
python3 tools/check-docs.py

step "tools/check-site.py"
python3 tools/check-site.py

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
    # install.sh is for Linux and macOS; nothing runs install.ps1 here or in CI.
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
