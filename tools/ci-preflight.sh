#!/usr/bin/env bash
# Usage: tools/ci-preflight.sh
#
# Fails early, with a readable message, when a self-hosted CI runner lacks something the build
# needs: a C compiler (libgit2 is built from source) and git itself
# (the tests drive the git executable).
set -euo pipefail

missing=()
command -v cc >/dev/null 2>&1 || missing+=("a C compiler (cc), e.g. sudo apt install build-essential")
command -v git >/dev/null 2>&1 || missing+=("git, e.g. sudo apt install git")
command -v cargo >/dev/null 2>&1 || missing+=("cargo (rustup)")

if ((${#missing[@]})); then
  echo "::error::This runner is missing build tools:" >&2
  for m in "${missing[@]}"; do echo "  - $m" >&2; done
  exit 1
fi

cc --version | head -n1
git --version
cargo --version
