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
# $CANOPY_SIM_ROOT, default $TMPDIR/canopy-usersim), a fixture
# repository with a local bare `origin` (so fetch, pull and push really run), builds the release
# binary and launches it. The first launch shows the real first-run questions; answer them with
# `x` like a user. `stop` quits the app, stops Xvfb and deletes the scratch folder.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
SIM=${CANOPY_SIM_ROOT:-${TMPDIR:-/tmp}/canopy-usersim}
# Where cargo puts the build, as tools/screenshot.sh finds it (a relative CARGO_TARGET_DIR is
# relative to the repository, where cargo runs).
TARGET=${CARGO_TARGET_DIR:-target}
[[ $TARGET == /* ]] || TARGET=$ROOT/$TARGET
BIN=$TARGET/release/canopy
STATE=$SIM/state

die() { echo "usersim: $*" >&2; exit 1; }
# The scratch folder's state is sourced and its Git configuration used, so it must be a folder of
# this user's that nobody else can write to (the default path is a predictable one in $TMPDIR).
private() {
  [[ -d $SIM && ! -L $SIM && -O $SIM ]] || die "$SIM is not a folder of yours"
  [[ -z $(find "$SIM" -maxdepth 0 \( -perm -020 -o -perm -002 \)) ]] || die "others can write to $SIM"
}
load() {
  private
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
    "$BIN" "$SIM/repo" >"$SIM/app.log" 2>&1 &
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
  [[ -e $SIM ]] || { mkdir -p -- "$(dirname -- "$SIM")" && mkdir -m 700 -- "$SIM"; }
  private
  mkdir -p "$SIM/home/.config" "$SIM/home/.local/share" "$SIM/home/.cache" "$SIM/tmp"
  printf '[user]\n\tname = Sim User\n\temail = sim@example.com\n' >"$SIM/home/.gitconfig"
  "$ROOT/tools/mkfixture.sh" --force "$SIM/repo" "$kind" >/dev/null
  git clone -q --bare "$SIM/repo" "$SIM/origin.git"
  # A fixture's own origin and its upstreams stay; only where it points changes. A fixture without
  # one (conflict, linear, big) gets the bare clone as its origin.
  if git -C "$SIM/repo" remote get-url origin >/dev/null 2>&1; then
    git -C "$SIM/repo" remote set-url origin "$SIM/origin.git"
  else
    git -C "$SIM/repo" remote add origin "$SIM/origin.git"
  fi
  git -C "$SIM/repo" fetch -q origin
  (cd "$ROOT" && cargo build -q --release -p canopy-app)
  # DISPLAY_BASE=N keeps this to displays :N to :N+9 (parallel runs each take a range); the
  # default is :70 to :99.
  base=${DISPLAY_BASE:-70} last=$((${DISPLAY_BASE:-70} + 9))
  [[ -n ${DISPLAY_BASE:-} ]] || last=99
  [[ $base =~ ^[0-9]+$ ]] || die "invalid DISPLAY_BASE: $base"
  for ((n = base; n <= last; n++)); do [[ -e /tmp/.X11-unix/X$n || -e /tmp/.X$n-lock ]] || break; done
  ((n <= last)) || die "no free X display from :$base to :$last"
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
