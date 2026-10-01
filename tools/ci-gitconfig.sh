#!/usr/bin/env bash
# Usage: tools/ci-gitconfig.sh <file>
#
# Writes a git configuration for the test suite to <file>, for use as GIT_CONFIG_GLOBAL. The
# self-hosted runners share one machine and one home directory, so CI never edits ~/.gitconfig.
set -euo pipefail

file=${1:?usage: tools/ci-gitconfig.sh <file>}
mkdir -p "$(dirname "$file")"
: > "$file"
git config --file "$file" user.name "Canopy CI"
git config --file "$file" user.email "ci@canopy.invalid"
git config --file "$file" init.defaultBranch main
git config --file "$file" commit.gpgsign false
git config --file "$file" tag.gpgsign false
