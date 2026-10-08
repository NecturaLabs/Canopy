#!/usr/bin/env bash
# Usage: tools/install-hooks.sh [--uninstall]
#
# Points this repository's core.hooksPath at tools/hooks, so the pre-commit hook there (clippy
# with warnings denied) runs before every commit in every worktree. --uninstall removes the setting.
set -euo pipefail

cd "$(dirname "$0")/.."

case "${1:-}" in
    --uninstall)
        git config --unset core.hooksPath || true
        echo "hooks uninstalled"
        ;;
    "")
        chmod +x tools/hooks/pre-commit
        git config core.hooksPath tools/hooks
        echo "hooks installed: core.hooksPath = tools/hooks"
        ;;
    *)
        echo "usage: tools/install-hooks.sh [--uninstall]" >&2
        exit 2
        ;;
esac
