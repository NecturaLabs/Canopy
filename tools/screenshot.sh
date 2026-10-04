#!/usr/bin/env bash
# Usage: tools/screenshot.sh <repo> <out.png> [--light] [--split] [--theme NAME] [--view history|changes|requests]
#                            [--overlay settings|privacy|cheatsheet|palette|consent|crash-consent|legal|licences|credential|host-key|merge|identity|settings-git|clear-identity|update-installed|update-running|trust-folder|diff-options]
#                            [--size WxH] [--zoom 0.8..1.6]
#
# <repo> may be "-" to render the welcome screen with no repository open.
#
# Renders Canopy headlessly on a virtual X server (Xvfb, software OpenGL via Mesa llvmpipe) with
# <repo> open, saves one frame to <out.png> once the repository has loaded, and prints the output
# path. Builds the release binary first (a no-op when it is up to date); set CANOPY_BIN to use a
# different binary and skip the build. Uses Xvfb when available, else a native Hyprland window
# on CANOPY_WORKSPACE (default 3). Output and native-run temporary files stay beside <out.png>.
# On Xvfb the app gets private data, config and cache folders and a fixed Git identity, so the
# image never depends on your own Canopy state or Git configuration.
set -euo pipefail

usage() { awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"; }

if [[ ${1:-} == -h || ${1:-} == --help ]]; then usage; exit 0; fi
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
    --overlay) extra+=(--overlay "${2:?--overlay needs a name, see --help}"); shift 2 ;;
    --zoom) extra+=(--zoom "${2:?--zoom needs a factor}"); shift 2 ;;
    --size) size=${2:?--size needs WxH}; shift 2 ;;
    -h | --help) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; usage; exit 2 ;;
  esac
done
if [[ ! $size =~ ^[0-9]+x[0-9]+$ ]]; then echo "invalid --size: $size (expected WxH)" >&2; exit 2; fi
repo_args=()
if [[ $repo != - ]]; then
  [[ -d $repo ]] || { echo "not a directory: $repo" >&2; exit 1; }
  repo_args=("$(realpath "$repo")")
fi
if ! command -v xvfb-run >/dev/null && [[ -z ${HYPRLAND_INSTANCE_SIGNATURE:-} ]]; then
  echo "screenshots need xvfb-run or a running Hyprland session" >&2
  exit 1
fi

root=$(cd "$(dirname "$0")/.." && pwd)
if [[ -z ${CANOPY_BIN:-} ]]; then
  cargo build --release -p canopy-app --quiet --manifest-path "$root/Cargo.toml"
  CANOPY_BIN="${CARGO_TARGET_DIR:-$root/target}/release/canopy"
fi

mkdir -p "$(dirname "$out")"
out_abs="$(cd "$(dirname "$out")" && pwd)/$(basename "$out")"
rm -f "$out_abs"

if command -v xvfb-run >/dev/null; then
  # Private data, config and cache folders and a fixed Git identity: the image must not depend on
  # (or write to) your own Canopy state, such as its saved window size, or your Git configuration.
  isolated=$(mktemp -d)
  trap 'rm -rf "$isolated"' EXIT
  printf '[user]\n\tname = Showcase Author\n\temail = author@example.com\n' > "$isolated/gitconfig"
  # Force X11 inside Xvfb even when called from a Wayland session.
  env -u WAYLAND_DISPLAY LIBGL_ALWAYS_SOFTWARE=1 \
    XDG_DATA_HOME="$isolated/data" XDG_CONFIG_HOME="$isolated/config" XDG_CACHE_HOME="$isolated/cache" \
    GIT_CONFIG_GLOBAL="$isolated/gitconfig" GIT_CONFIG_NOSYSTEM=1 \
    timeout 60 xvfb-run -a -s "-screen 0 ${size}x24" \
    "$CANOPY_BIN" --screenshot "$out_abs" --size "$size" "${extra[@]}" "${repo_args[@]}"
else
  workspace=${CANOPY_WORKSPACE:-3}
  [[ $workspace =~ ^[1-9][0-9]*$ ]] || { echo "invalid CANOPY_WORKSPACE: $workspace" >&2; exit 2; }
  native_dir=$(mktemp -d "$(dirname "$out_abs")/.canopy-shot.XXXXXX")
  trap 'rm -f "$native_dir/run.sh" "$native_dir/run.log" "$native_dir/status"; rmdir "$native_dir"' EXIT
  CANOPY_BIN=$(realpath "$CANOPY_BIN")
  command=(env)
  if [[ -z ${WAYLAND_DISPLAY:-} ]]; then command+=(-u WAYLAND_DISPLAY); fi
  for name in XDG_CONFIG_HOME XDG_DATA_HOME XDG_CACHE_HOME TMPDIR CANOPY_HOSTING_API_BASE; do
    if [[ -v $name ]]; then command+=("$name=${!name}"); fi
  done
  command+=(timeout 60 "$CANOPY_BIN" --screenshot "$out_abs" --size "$size" "${extra[@]}" "${repo_args[@]}")
  {
    printf '#!/usr/bin/env bash\n'
    printf '%q ' "${command[@]}"
    printf '> %q 2>&1\n' "$native_dir/run.log"
    printf 'printf "%%s\\n" "$?" > %q\n' "$native_dir/status"
  } > "$native_dir/run.sh"
  python3 - "$workspace" "$size" "$native_dir/run.sh" <<'PYTHON'
import json, shlex, subprocess, sys
workspace, size, runner = sys.argv[1:]
command = f"[workspace {workspace} silent; float; size {size.replace('x', ' ')}; center; renderunfocused] " + shlex.join(["bash", runner])
subprocess.run(["hyprctl", "eval", "hl.exec_cmd(" + json.dumps(command, ensure_ascii=False) + ")"], check=True, stdout=subprocess.DEVNULL)
PYTHON
  for ((i=0; i<650; i++)); do
    [[ -f $native_dir/status ]] && break
    sleep 0.1
  done
  [[ -f $native_dir/status ]] || { echo "native screenshot timed out" >&2; exit 1; }
  status=$(cat "$native_dir/status")
  [[ $status == 0 ]] || { cat "$native_dir/run.log" >&2; exit "$status"; }
fi

[[ -s $out_abs ]] || { echo "no screenshot was written" >&2; exit 1; }
echo "$out_abs"
