#!/usr/bin/env bash
# Usage: tools/usersim.sh start [--kind KIND] [--size WxH]
#        tools/usersim.sh x XDOTOOL-ARGS...    (e.g. x mousemove 300 140 click 1)
#        tools/usersim.sh shot OUT.png
#        tools/usersim.sh restart               (quit with Ctrl+Q and launch again, same state)
#        tools/usersim.sh stop
#
# The user simulation review (docs/playbook.md): Canopy driven like a user on a private Xvfb
# display, never on the desktop session, with its own home, XDG folders and Git configuration in
# a scratch folder, on fixture repositories only. `start` makes the scratch folder (under
# $CANOPY_SIM_ROOT, default /run/media/nectura/Programming SSD/tmp/canopy-usersim), a fixture
# repository with a local bare `origin` (so fetch, pull and push really run), builds the release
# binary and launches it. The first launch shows the real first-run questions; answer them with
# `x` like a user. `stop` quits the app, stops Xvfb and deletes the scratch folder.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
SIM=${CANOPY_SIM_ROOT:-/run/media/nectura/Programming SSD/tmp/canopy-usersim}
STATE=$SIM/state

die() { echo "usersim: $*" >&2; exit 1; }
load() {
  [[ -f $STATE ]] || die "not started (tools/usersim.sh start)"
  # shellcheck source=/dev/null
  source "$STATE"
}

launch() {
  env -i PATH="$PATH" HOME="$SIM/home" USER="${USER:-sim}" LANG=C.UTF-8 \
    XDG_DATA_HOME="$SIM/home/.local/share" XDG_CONFIG_HOME="$SIM/home/.config" \
    XDG_CACHE_HOME="$SIM/home/.cache" XDG_STATE_HOME="$SIM/home/.local/state" \
    TMPDIR="$SIM/tmp" GIT_CONFIG_GLOBAL="$SIM/home/.gitconfig" GIT_CONFIG_NOSYSTEM=1 \
    DISPLAY="$DISPLAY_NO" LIBGL_ALWAYS_SOFTWARE=1 CANOPY_LOG="${CANOPY_LOG:-canopy=info}" \
    "$ROOT/target/release/canopy" "$SIM/repo" >"$SIM/app.log" 2>&1 &
  echo "APP_PID=$!" >>"$STATE"
}

case ${1:-} in
start)
  shift
  kind=showcase size=1440x900
  while [[ $# -gt 0 ]]; do
    case $1 in
      --kind) kind=$2; shift 2 ;;
      --size) size=$2; shift 2 ;;
      *) die "unknown option $1" ;;
    esac
  done
  [[ -e $STATE ]] && die "already running; tools/usersim.sh stop first"
  for tool in Xvfb xdotool import; do
    command -v "$tool" >/dev/null || die "needs $tool"
  done
  mkdir -p "$SIM/home/.config" "$SIM/home/.local/share" "$SIM/home/.cache" "$SIM/tmp"
  printf '[user]\n\tname = Sim User\n\temail = sim@example.com\n' >"$SIM/home/.gitconfig"
  "$ROOT/tools/mkfixture.sh" --force "$SIM/repo" "$kind" >/dev/null
  git clone -q --bare "$SIM/repo" "$SIM/origin.git"
  # The fixture's own origin and its upstreams stay; only where it points changes.
  git -C "$SIM/repo" remote set-url origin "$SIM/origin.git"
  git -C "$SIM/repo" fetch -q origin
  (cd "$ROOT" && cargo build -q --release -p canopy-app)
  for n in $(seq 70 99); do [[ -e /tmp/.X11-unix/X$n || -e /tmp/.X$n-lock ]] || break; done
  DISPLAY_NO=:$n
  Xvfb "$DISPLAY_NO" -screen 0 "${size}x24" -nolisten tcp >"$SIM/xvfb.log" 2>&1 &
  printf 'DISPLAY_NO=%s\nXVFB_PID=%s\n' "$DISPLAY_NO" "$!" >"$STATE"
  sleep 1
  launch
  echo "usersim: Canopy on $DISPLAY_NO, scratch $SIM (log: app.log)"
  ;;
x)
  shift
  load
  # Xvfb has no window manager to give the window keyboard focus; without it keys go nowhere.
  DISPLAY=$DISPLAY_NO xdotool search --name Canopy windowfocus 2>/dev/null || true
  DISPLAY=$DISPLAY_NO xdotool "$@"
  ;;
shot)
  load
  [[ -n ${2:-} ]] || die "shot OUT.png"
  DISPLAY=$DISPLAY_NO import -window root "$2"
  echo "$2"
  ;;
restart)
  load
  DISPLAY=$DISPLAY_NO xdotool search --name Canopy windowfocus key ctrl+q 2>/dev/null || true
  for _ in $(seq 50); do kill -0 "$APP_PID" 2>/dev/null || break; sleep 0.1; done
  kill "$APP_PID" 2>/dev/null || true
  sed -i '/^APP_PID=/d' "$STATE"
  launch
  ;;
stop)
  load
  DISPLAY=$DISPLAY_NO xdotool search --name Canopy windowfocus key ctrl+q 2>/dev/null || true
  for _ in $(seq 50); do kill -0 "$APP_PID" 2>/dev/null || break; sleep 0.1; done
  kill "$APP_PID" "$XVFB_PID" 2>/dev/null || true
  rm -rf -- "$SIM"
  echo "usersim: stopped, $SIM removed"
  ;;
*)
  sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
  exit 2
  ;;
esac
