#!/usr/bin/env bash
# Usage: tools/screenshot.sh <repo> <out.png> [--light] [--split] [--theme NAME] [--view history|changes|requests]
#                            [--overlay settings|privacy|cheatsheet|palette|consent|consent-updated|crash-consent|legal|credential|host-key|merge|identity|settings-git|settings-git-pull|settings-git-commits|settings-git-config|clear-identity|update-installed|update-running|trust-prompt|trust-banner|trust-safety-off|privacy-trust-off|diff-options|settings-integrations|settings-integrations-custom|settings-accounts|settings-updates|settings-notifications|settings-keyboard|settings-backup|settings-repository|settings-repository-pull|settings-repository-diffs|settings-search|settings-search-none|about|repository-insights|file-history|file-history-commit|lfs|lfs-patterns|lfs-locks|search-changes|clean-stale|author-card|request-filters|request-sort|commit-expanded|commit-compact|amend-commit|toasts|notifications|branch-selected|long-message|gallery|menu-branch|menu-commit|menu-file|menu-lfs-file|blame|<dialog>]
#                            [--style standard|deeper] [--density comfortable|dense] [--request-density comfortable|dense]
#                            [--translucent on|off]
#                            [--contrast on|off] [--custom-theme SPEC]
#                            [--motion system|reduced|full] [--gallery-page N]
#                            [--demo-requests SCENARIO] [--size WxH] [--zoom 0.8..1.6]
#
# <dialog> is any dialog overlay the app lists in its own --help (open, clone-error, confirm, terms,
# approve, ...).
#
# menu-* opens a context menu by a right click once the window has settled: the feature/auth
# branch row's, the top commit row's or the src/main.rs changed file row's (the
# showcase fixture's names).
#
# blame clicks the Blame button of the file the Changes view selects first, and shows its blame.
#
# The repository is trusted, so the picture shows the view and not the Trust prompt. trust-banner
# shows it as an untrusted one (read-only banner, prompt answered with Don't trust); trust-prompt
# is the prompt over a fixed illustrative path; privacy-trust-off is Settings with trust safety off.
# CANOPY_SCREENSHOT_TRUSTED=<dir> in the environment also trusts <dir>, for what a trusted folder
# shows (a signature, say).
#
# <repo> may be "-" to render the welcome screen with no repository open.
#
# --demo-requests fills the pull/merge requests view with invented requests, with no network:
# list, list-all, list-closed, list-unchecked, list-overflow, list-more, list-search, filtered, no-match, a request number
# (428, 427, 425, 422, 420, ...), busy, loading, details-loading, details-error, error, no-token or
# empty; prefix gitlab: for merge requests.
# It implies --view requests and needs a repository without a GitHub or GitLab remote (the
# fixtures have none).
#
# Renders Canopy headlessly on a virtual X server (Xvfb, software OpenGL via Mesa llvmpipe) with
# <repo> open, saves one frame to <out.png> once the repository has loaded, and prints the output
# path. Builds the release binary first (a no-op when it is up to date); set CANOPY_BIN to use a
# different binary and skip the build. Uses Xvfb when available, else a native Hyprland window
# on CANOPY_WORKSPACE (default 3). DISPLAY_BASE=N keeps Xvfb to displays :N to :N+9. Output and native-run temporary files stay beside <out.png>.
# On Xvfb the app gets private home, data, config and cache folders, a fixed Git identity and no
# TERMINAL, EDITOR or VISUAL, so the image never depends on your own Canopy state, Git
# configuration or programs; a screenshot also ignores the pointer (no hover, no tooltip).
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
    --style) extra+=(--style "${2:?--style needs standard or deeper}"); shift 2 ;;
    --density) extra+=(--density "${2:?--density needs comfortable or dense}"); shift 2 ;;
    --request-density) extra+=(--request-density "${2:?--request-density needs comfortable or dense}"); shift 2 ;;
    --translucent) extra+=(--translucent "${2:?--translucent needs on or off}"); shift 2 ;;
    --contrast) extra+=(--contrast "${2:?--contrast needs on or off}"); shift 2 ;;
    --custom-theme) extra+=(--custom-theme "${2:?--custom-theme needs a list, see --help}"); shift 2 ;;
    --motion) extra+=(--motion "${2:?--motion needs system, reduced or full}"); shift 2 ;;
    --gallery-page) extra+=(--gallery-page "${2:?--gallery-page needs a number}"); shift 2 ;;
    --demo-requests) extra+=(--demo-requests "${2:?--demo-requests needs a scenario, see --help}"); shift 2 ;;
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

# Both display backends must keep screenshots clear of the user's Canopy and Git state.
isolated=$(mktemp -d)
trap 'rm -rf "$isolated"' EXIT
mkdir -p "$isolated/home" "$isolated/tmp"
printf '[user]\n\tname = Showcase Author\n\temail = author@example.com\n' > "$isolated/gitconfig"

if command -v xvfb-run >/dev/null; then
  # DISPLAY_BASE=N takes the first free X display from :N to :N+9 (parallel runs keep to their
  # own range); without it xvfb-run picks one itself.
  server=(-a)
  if [[ -n ${DISPLAY_BASE:-} ]]; then
    [[ $DISPLAY_BASE =~ ^[0-9]+$ ]] || { echo "invalid DISPLAY_BASE: $DISPLAY_BASE" >&2; exit 2; }
    for ((n = DISPLAY_BASE; n < DISPLAY_BASE + 10; n++)); do
      [[ -e /tmp/.X11-unix/X$n || -e /tmp/.X$n-lock ]] || break
    done
    ((n < DISPLAY_BASE + 10)) || { echo "no free X display from :$DISPLAY_BASE to :$((DISPLAY_BASE + 9))" >&2; exit 1; }
    server=(-n "$n")
  fi
  # Private home, data, config and cache folders, a fixed Git identity and no TERMINAL, EDITOR or
  # VISUAL: the image must not depend on (or write to) your own Canopy state, such as its saved
  # window size, your Git configuration or your programs (Settings > Integrations names them),
  # and a path under "~" must not show your own home folder.
  # Force X11 inside Xvfb even when called from a Wayland session.
  env -u WAYLAND_DISPLAY -u TERMINAL -u EDITOR -u VISUAL LIBGL_ALWAYS_SOFTWARE=1 CANOPY_SCREENSHOT_ISOLATED=1 HOME="$isolated/home" \
    XDG_DATA_HOME="$isolated/data" XDG_CONFIG_HOME="$isolated/config" XDG_CACHE_HOME="$isolated/cache" \
    GIT_CONFIG_GLOBAL="$isolated/gitconfig" GIT_CONFIG_NOSYSTEM=1 \
    timeout 60 xvfb-run "${server[@]}" -s "-screen 0 ${size}x24" \
    "$CANOPY_BIN" --screenshot "$out_abs" --size "$size" "${extra[@]}" "${repo_args[@]}"
else
  workspace=${CANOPY_WORKSPACE:-3}
  [[ $workspace =~ ^[1-9][0-9]*$ ]] || { echo "invalid CANOPY_WORKSPACE: $workspace" >&2; exit 2; }
  native_dir=$(mktemp -d "$(dirname "$out_abs")/.canopy-shot.XXXXXX")
  trap 'rm -f "$native_dir/run.sh" "$native_dir/run.log" "$native_dir/status"; rmdir "$native_dir"; rm -rf "$isolated"' EXIT
  CANOPY_BIN=$(realpath "$CANOPY_BIN")
  command=(env -u TERMINAL -u EDITOR -u VISUAL)
  if [[ -z ${WAYLAND_DISPLAY:-} ]]; then command+=(-u WAYLAND_DISPLAY); fi
  if [[ -v CANOPY_HOSTING_API_BASE ]]; then
    command+=("CANOPY_HOSTING_API_BASE=$CANOPY_HOSTING_API_BASE")
  fi
  if [[ -v CANOPY_SCREENSHOT_TRUSTED ]]; then
    command+=("CANOPY_SCREENSHOT_TRUSTED=$CANOPY_SCREENSHOT_TRUSTED")
  fi
  command+=(CANOPY_SCREENSHOT_ISOLATED=1 HOME="$isolated/home" \
    XDG_DATA_HOME="$isolated/data" XDG_CONFIG_HOME="$isolated/config" \
    XDG_CACHE_HOME="$isolated/cache" TMPDIR="$isolated/tmp" \
    GIT_CONFIG_GLOBAL="$isolated/gitconfig" GIT_CONFIG_NOSYSTEM=1)
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
command = f"[workspace {workspace} silent; float; size {size.replace('x', ' ')}; center; render_unfocused] " + shlex.join(["bash", runner])
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
