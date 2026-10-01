#!/usr/bin/env bash
# Usage: tools/screenshot.sh <repo> <out.png> [--light] [--split] [--theme NAME] [--view history|changes|requests]
#                            [--overlay settings|cheatsheet|palette] [--size WxH]
#
# <repo> may be "-" to render the welcome screen with no repository open.
#
# Renders Canopy headlessly on a virtual X server (Xvfb, software OpenGL via Mesa llvmpipe) with
# <repo> open, saves one frame to <out.png> once the repository has loaded, and prints the output
# path. Builds the release binary first (a no-op when it is up to date); set CANOPY_BIN to use a
# different binary and skip the build. Requires xvfb-run (Debian/Ubuntu: `apt install xvfb`).
set -euo pipefail

usage() { awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"; }

if [[ $# -lt 2 ]]; then usage; exit 2; fi
repo=$1 out=$2
shift 2

size=1440x900
extra=()
while [[ $# -gt 0 ]]; do
  case $1 in
    --light) extra+=(--light); shift ;;
    --split) extra+=(--split); shift ;;
    --theme) extra+=(--theme "${2:?--theme needs a name}"); shift 2 ;;
    --view) extra+=(--view "${2:?--view needs history, changes or requests}"); shift 2 ;;
    --overlay) extra+=(--overlay "${2:?--overlay needs settings, cheatsheet or palette}"); shift 2 ;;
    --size) size=${2:?--size needs WxH}; shift 2 ;;
    -h | --help) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; usage; exit 2 ;;
  esac
done
if [[ ! $size =~ ^[0-9]+x[0-9]+$ ]]; then echo "invalid --size: $size (expected WxH)" >&2; exit 2; fi
repo_args=()
if [[ $repo != - ]]; then
  [[ -d $repo ]] || { echo "not a directory: $repo" >&2; exit 1; }
  repo_args=("$repo")
fi
command -v xvfb-run >/dev/null || { echo "xvfb-run not found (apt install xvfb)" >&2; exit 1; }

root=$(cd "$(dirname "$0")/.." && pwd)
if [[ -z ${CANOPY_BIN:-} ]]; then
  cargo build --release -p canopy-app --quiet --manifest-path "$root/Cargo.toml"
  CANOPY_BIN="$root/target/release/canopy"
fi

mkdir -p "$(dirname "$out")"
out_abs="$(cd "$(dirname "$out")" && pwd)/$(basename "$out")"
rm -f "$out_abs"

# Force X11 inside Xvfb even when called from a Wayland session.
env -u WAYLAND_DISPLAY LIBGL_ALWAYS_SOFTWARE=1 \
  timeout 60 xvfb-run -a -s "-screen 0 ${size}x24" \
  "$CANOPY_BIN" --screenshot "$out_abs" --size "$size" "${extra[@]}" "${repo_args[@]}"

[[ -s $out_abs ]] || { echo "no screenshot was written" >&2; exit 1; }
echo "$out_abs"
