#!/usr/bin/env bash
# Usage: tools/stress.sh <clone|open|churn|report> [options]     (tools/stress.sh --help)
#
# Stress-test harness for Canopy: 100 public repositories open at once, one per tab.
# The procedure, scenarios and pass criteria are in docs/stress-test.md.
#
#   clone   Clone the repositories listed in tools/stress-repos.txt into a cache folder
#   open    Launch Canopy on the cached clones and sample its resource use until it exits
#   churn   Mutate cached clones (files, commits, branch switches) while Canopy watches them
#   report  Summarise a run's JSON log and sampler CSV (tools/stress_report.py)
#
# Everything lives under two folders, and nothing outside them is ever modified:
#   cache    $CANOPY_STRESS_DIR or --dir          default ${XDG_CACHE_HOME:-~/.cache}/canopy-stress
#   results  $CANOPY_STRESS_RESULTS or --results  default ${XDG_STATE_HOME:-~/.local/state}/canopy-stress
# `churn` refuses any repository that is not a clone made by `clone` inside the cache folder.
# Linux only for `open` (it reads /proc). Needs bash 4.3+, GNU coreutils and git.
set -euo pipefail
export LC_ALL=C

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(dirname "$SCRIPT_DIR")

usage() {
  cat <<'EOF'
Usage: tools/stress.sh <command> [options]

Commands
  clone    Clone the listed repositories into the cache folder (resumable: finished clones
           are skipped, an interrupted one is removed and redone).
  open     Launch Canopy with every cached clone as a tab, with detailed logging, and sample
           RSS, threads, file descriptors, inotify watches and CPU every second to a CSV.
  churn    Mutate cached clones concurrently so Canopy's file watchers fire everywhere.
  report   Print a summary of a run (calls tools/stress_report.py).

Selection (clone, open, churn)
  --list FILE         Repository list (default: tools/stress-repos.txt)
  --subset N          Evenly spaced N entries of the (filtered) list, so every size class is
                      represented. Applies to clone and open; for churn it is --repos
  --class A,B         Only these size classes: monstrous, large, medium, small
  --exclude R,R       Skip these owner/repo entries
  --dir DIR           Cache folder (or $CANOPY_STRESS_DIR)
  --results DIR       Results folder (or $CANOPY_STRESS_RESULTS)

clone
  --shallow           Clone with --depth 1 (history is gone: only for checking tree size)
  --depth N           Shallow depth (implies --shallow)
  --blobless          Clone with --filter=blob:none (full history, file contents on demand;
                      libgit2 cannot fetch them, so old diffs may fail: see docs/stress-test.md)
  --shallow-for R,R   Clone just these entries shallow, e.g. chromium/chromium
  --submodules        Also initialise submodules (large; off by default)
  --jobs N            Parallel clones (default 4)
  --retries N         Retries per clone after a failure (default 2)
  --base-url URL      Where repositories come from (default https://github.com; or
                      $CANOPY_STRESS_BASE_URL, e.g. a local mirror)
  --min-free-gb N     Refuse to start with less free space than this (default 5)
  --dry-run           Print what would be cloned and the free space, then stop

open
  --bin FILE          Canopy binary (or $CANOPY_BIN, default target/release/canopy)
  --log-filter F      CANOPY_LOG value (default: the Detailed level's filter)
  --duration SEC      Stop Canopy (SIGTERM) after this long; default: until it exits
  --sample-interval S Sampling period in seconds (default 1)
  --label TEXT        Free text stored in the run's meta.txt (e.g. the scenario)
  --real-data         Use the real Canopy data folder instead of an isolated one
                      (settings, tabs and logs then mix with your own)
  Each run gets $RESULTS/run-<UTC time>/ with meta.txt, samples.csv, canopy.jsonl (copy of
  the app's log), stderr.txt and crashes.txt; $RESULTS/latest points to it.

churn
  --repos N           Number of repositories to mutate at once (default 20, evenly spread)
  --duration SEC      How long to churn (default 300)
  --interval SEC      Mean pause between actions in one repository (default 3)
  --files N           Most files touched by one file action (default 10)
  --seed N            Random seed (default 1)
  --deep              Also check out commits far back in history (full clones only)
  --modify-many N     Once: append a line to N tracked files in each selected repository
                      (thousands of modified files for the Changes view), then stop
  --conflict-setup    Once: prepare stress/conflict-a and stress/conflict-b in each selected
                      repository so that merging b into a conflicts, then stop
  --leave-merging     With --conflict-setup: also run the merge, leaving it in progress
  --restore           Once: put every touched clone back on its original branch and remove
                      what churn created (branches and tags named stress/*, stress-churn/)
  --log FILE          Action log (default: $RESULTS/latest/churn-<time>.log)
  Churn only ever uses local git commands (no network) and skips a repository while a merge,
  rebase or cherry-pick is in progress in it.

report
  --run DIR           A run folder (default: $RESULTS/latest)
  --log FILE --csv FILE --churn FILE   Individual inputs instead of --run
  Other options are passed to tools/stress_report.py (see its --help).

Environment: CANOPY_STRESS_DIR, CANOPY_STRESS_RESULTS, CANOPY_BIN, CANOPY_STRESS_BASE_URL.
EOF
}

die() { echo "stress: $*" >&2; exit 1; }
info() { echo "stress: $*" >&2; }

if ((BASH_VERSINFO[0] < 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] < 3))); then
  die "bash 4.3 or newer is required (this is $BASH_VERSION)"
fi

# ---------------------------------------------------------------------------------------
# Options
# ---------------------------------------------------------------------------------------
CMD=${1:-}
if [[ -z $CMD || $CMD == -h || $CMD == --help || $CMD == help ]]; then usage; exit 0; fi
shift

LIST=$SCRIPT_DIR/stress-repos.txt
DIR=${CANOPY_STRESS_DIR:-}
RESULTS=${CANOPY_STRESS_RESULTS:-}
BIN=${CANOPY_BIN:-}
BASE_URL=${CANOPY_STRESS_BASE_URL:-https://github.com}
SUBSET=0 CLASSES="" EXCLUDE=""
SHALLOW=0 DEPTH=1 BLOBLESS=0 SHALLOW_FOR="" SUBMODULES=0 JOBS=4 RETRIES=2 MIN_FREE_GB=5 DRY=0
LOG_FILTER='info,canopy=debug,canopy_core=debug,canopy_hosting=debug'
DURATION="" SAMPLE_INTERVAL=1 LABEL="" REAL_DATA=0
REPOS=20 INTERVAL=3 FILES=10 SEED=1 DEEP=0 MODIFY_MANY=0 CONFLICT=0 LEAVE_MERGING=0 RESTORE=0
CHURN_LOG="" RUN_DIR="" REPORT_LOG="" REPORT_CSV="" REPORT_CHURN=""
PASSTHROUGH=()

need_arg() { [[ $# -ge 2 ]] || die "$1 needs a value"; }
need_int() { [[ $2 =~ ^[0-9]+$ ]] || die "$1 needs a whole number, got '$2'"; }
need_num() { [[ $2 =~ ^[0-9]+([.][0-9]+)?$ ]] || die "$1 needs a number, got '$2'"; }

while (($#)); do
  case $1 in
    -h | --help) usage; exit 0 ;;
    --list) need_arg "$@"; LIST=$2; shift 2 ;;
    --dir) need_arg "$@"; DIR=$2; shift 2 ;;
    --results) need_arg "$@"; RESULTS=$2; shift 2 ;;
    --subset) need_arg "$@"; need_int "$1" "$2"; SUBSET=$2; shift 2 ;;
    --class) need_arg "$@"; CLASSES=$2; shift 2 ;;
    --exclude) need_arg "$@"; EXCLUDE=${EXCLUDE:+$EXCLUDE,}$2; shift 2 ;;
    --shallow) SHALLOW=1; shift ;;
    --depth) need_arg "$@"; need_int "$1" "$2"; SHALLOW=1; DEPTH=$2; shift 2 ;;
    --blobless) BLOBLESS=1; shift ;;
    --shallow-for) need_arg "$@"; SHALLOW_FOR=${SHALLOW_FOR:+$SHALLOW_FOR,}$2; shift 2 ;;
    --submodules) SUBMODULES=1; shift ;;
    --jobs) need_arg "$@"; need_int "$1" "$2"; JOBS=$2; shift 2 ;;
    --retries) need_arg "$@"; need_int "$1" "$2"; RETRIES=$2; shift 2 ;;
    --base-url) need_arg "$@"; BASE_URL=${2%/}; shift 2 ;;
    --min-free-gb) need_arg "$@"; need_int "$1" "$2"; MIN_FREE_GB=$2; shift 2 ;;
    --dry-run) DRY=1; shift ;;
    --bin) need_arg "$@"; BIN=$2; shift 2 ;;
    --log-filter) need_arg "$@"; LOG_FILTER=$2; shift 2 ;;
    --duration) need_arg "$@"; need_int "$1" "$2"; DURATION=$2; shift 2 ;;
    --sample-interval) need_arg "$@"; need_num "$1" "$2"; SAMPLE_INTERVAL=$2; shift 2 ;;
    --label) need_arg "$@"; LABEL=$2; shift 2 ;;
    --real-data) REAL_DATA=1; shift ;;
    --repos) need_arg "$@"; need_int "$1" "$2"; REPOS=$2; shift 2 ;;
    --interval) need_arg "$@"; need_num "$1" "$2"; INTERVAL=$2; shift 2 ;;
    --files) need_arg "$@"; need_int "$1" "$2"; FILES=$2; shift 2 ;;
    --seed) need_arg "$@"; need_int "$1" "$2"; SEED=$2; shift 2 ;;
    --deep) DEEP=1; shift ;;
    --modify-many) need_arg "$@"; need_int "$1" "$2"; MODIFY_MANY=$2; shift 2 ;;
    --conflict-setup) CONFLICT=1; shift ;;
    --leave-merging) LEAVE_MERGING=1; shift ;;
    --restore) RESTORE=1; shift ;;
    --log) need_arg "$@"; CHURN_LOG=$2; REPORT_LOG=$2; shift 2 ;;
    --run) need_arg "$@"; RUN_DIR=$2; shift 2 ;;
    --csv) need_arg "$@"; REPORT_CSV=$2; shift 2 ;;
    --churn) need_arg "$@"; REPORT_CHURN=$2; shift 2 ;;
    *)
      if [[ $CMD == report ]]; then PASSTHROUGH+=("$1"); shift; else die "unknown option $1 (see --help)"; fi
      ;;
  esac
done

((SHALLOW && BLOBLESS)) && die "--shallow and --blobless are alternatives; pick one"

DIR=${DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/canopy-stress}
RESULTS=${RESULTS:-${XDG_STATE_HOME:-$HOME/.local/state}/canopy-stress}
DIR=$(realpath -m -- "$DIR")
RESULTS=$(realpath -m -- "$RESULTS")
case $DIR in
  / | "$HOME" | "$(dirname "$HOME")") die "refusing to use $DIR as the cache folder" ;;
esac

MARKER_CACHE=.canopy-stress-cache # in the cache folder: only folders with it are ever modified
MARKER_REPO=canopy-stress         # in <clone>/.git: only clones made by `clone` are ever modified

# ---------------------------------------------------------------------------------------
# The repository list
# ---------------------------------------------------------------------------------------
valid_repo() {
  [[ $1 =~ ^[A-Za-z0-9_][A-Za-z0-9_.-]*/[A-Za-z0-9_][A-Za-z0-9_.-]*$ ]]
}

in_csv() { # in_csv "a,b,c" b
  [[ ,$1, == *,"$2",* ]]
}

check_list() {
  [[ -r $LIST ]] || die "cannot read the repository list $LIST"
  awk '
    /^[[:space:]]*(#|$)/ { next }
    { sub(/#.*/, "") }
    NF != 2 { printf "line %d: expected `owner/repo class`: %s\n", NR, $0; bad = 1; next }
    $2 !~ /^(monstrous|large|medium|small)$/ { printf "line %d: unknown class %s\n", NR, $2; bad = 1 }
    $1 in seen { printf "line %d: duplicate %s\n", NR, $1; bad = 1 }
    { seen[$1] = 1 }
    END { exit bad }
  ' "$LIST" >&2 || die "fix $LIST"
}

# Prints "owner/repo class" lines: filtered by class and exclude list, then (if $1 is a number
# above zero and below the count) reduced to that many evenly spaced entries.
select_repos() {
  local n=${1:-0}
  awk -v classes="$CLASSES" -v ex="$EXCLUDE" -v n="$n" '
    BEGIN {
      nc = split(classes, C, ","); ne = split(ex, E, ",")
      for (i = 1; i <= nc; i++) okc[C[i]] = 1
      for (i = 1; i <= ne; i++) bad[E[i]] = 1
    }
    /^[[:space:]]*(#|$)/ { next }
    { sub(/#.*/, "") }
    NF < 2 { next }
    nc && !($2 in okc) { next }
    ($1 in bad) { next }
    { a[++t] = $1 " " $2 }
    END {
      if (n > 0 && n < t) { for (i = 0; i < n; i++) print a[int(i * t / n) + 1] }
      else { for (i = 1; i <= t; i++) print a[i] }
    }
  ' "$LIST"
}

SEL=() # "owner/repo class" entries
load_selection() {
  local n=${1:-0} line
  check_list
  SEL=()
  while IFS= read -r line; do
    valid_repo "${line%% *}" || die "bad repository name in $LIST: $line"
    SEL+=("$line")
  done < <(select_repos "$n")
  ((${#SEL[@]})) || die "no repositories selected (check --class, --exclude and $LIST)"
}

# ---------------------------------------------------------------------------------------
# Cache folder safety
# ---------------------------------------------------------------------------------------
cache_ready() {
  [[ -f $DIR/$MARKER_CACHE ]] || die "$DIR is not a stress cache (run 'clone' first, or pass --dir)"
}

is_cached() { [[ -f $DIR/$1/.git/$MARKER_REPO && ! -L $DIR/$1/.git ]]; }

# Succeeds only for owner/repo-shaped directories inside the cache that `clone` made.
# Every mutation goes through this check.
safe_repo() {
  local p rel
  p=$(realpath -e -- "$1" 2>/dev/null) || return 1
  case $p in "$DIR"/*) ;; *) return 1 ;; esac
  rel=${p#"$DIR"/}
  [[ $rel == */* && $rel != */*/* ]] || return 1
  [[ -d $p/.git && ! -L $p/.git && -f $p/.git/$MARKER_REPO ]]
}

# git with no user config, no hooks, no network, no gc, and nothing that prompts.
# It refuses to run outside a safe clone.
sgit() {
  local dir=$1
  shift
  safe_repo "$dir" || {
    echo "REFUSED: $dir is not a stress clone inside $DIR" >&2
    return 99
  }
  GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_TERMINAL_PROMPT=0 \
    GIT_ALLOW_PROTOCOL=none GIT_NO_LAZY_FETCH=1 GIT_LFS_SKIP_SMUDGE=1 \
    GIT_AUTHOR_NAME="Canopy Stress" GIT_AUTHOR_EMAIL="stress@canopy.invalid" \
    GIT_COMMITTER_NAME="Canopy Stress" GIT_COMMITTER_EMAIL="stress@canopy.invalid" \
    git -C "$dir" -c core.hooksPath=/dev/null -c gc.auto=0 -c maintenance.auto=false \
    -c commit.gpgsign=false -c tag.gpgsign=false -c core.fsmonitor=false \
    -c advice.detachedHead=false "$@"
}

free_gb() { df -Pk "$1" | awk 'NR == 2 { printf "%d", $4 / 1048576 }'; }

# ---------------------------------------------------------------------------------------
# clone
# ---------------------------------------------------------------------------------------
STATUS=""

clone_one() {
  local repo=$1 idx=$2 total=$3 class=$4
  local dest=$DIR/$repo mode=full try=0 t0=$SECONDS
  local errlog=$DIR/.logs/${repo//\//__}.err
  local -a args=(clone --quiet)

  if is_cached "$repo"; then
    echo "[$idx/$total] skip   $repo (already cloned)"
    echo "skip $repo" >>"$STATUS"
    return 0
  fi
  if ((SHALLOW)) || in_csv "$SHALLOW_FOR" "$repo"; then
    mode=shallow
    args+=(--depth "$DEPTH")
  elif ((BLOBLESS)); then
    mode=blobless
    args+=(--filter=blob:none)
  fi
  if ((SUBMODULES)); then
    args+=(--recurse-submodules)
    [[ $mode == shallow ]] && args+=(--shallow-submodules)
  fi

  echo "[$idx/$total] clone  $repo ($class, $mode)"
  while :; do
    # A folder without the marker is the remains of an interrupted clone of ours.
    if [[ -e $dest ]]; then
      [[ $dest == "$DIR"/*/* ]] || die "unexpected clone path $dest"
      rm -rf -- "$dest"
    fi
    mkdir -p -- "$DIR/${repo%/*}"
    if GIT_TERMINAL_PROMPT=0 git -c http.lowSpeedLimit=1000 -c http.lowSpeedTime=120 \
      "${args[@]}" "$BASE_URL/$repo.git" "$dest" 2>"$errlog"; then
      printf 'mode=%s\ncloned=%s\nclass=%s\n' "$mode" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$class" \
        >"$dest/.git/$MARKER_REPO"
      echo "[$idx/$total] done   $repo in $((SECONDS - t0))s"
      echo "ok $repo" >>"$STATUS"
      return 0
    fi
    if ((try >= RETRIES)); then
      echo "[$idx/$total] FAILED $repo: $(tail -n 1 "$errlog" 2>/dev/null)" >&2
      rm -rf -- "$dest"
      echo "fail $repo" >>"$STATUS"
      return 0
    fi
    try=$((try + 1))
    echo "[$idx/$total] retry  $repo ($try of $RETRIES): $(tail -n 1 "$errlog" 2>/dev/null)" >&2
    sleep 5
  done
}

cmd_clone() {
  command -v git >/dev/null || die "git is not installed"
  load_selection "$SUBSET"
  local total=${#SEL[@]} free i=0 line

  if [[ -e $DIR && ! -f $DIR/$MARKER_CACHE ]]; then
    if [[ -n $(ls -A -- "$DIR" 2>/dev/null) ]]; then
      die "$DIR exists, is not empty and is not a stress cache; use another --dir"
    fi
  fi
  local probe=$DIR
  while [[ ! -d $probe ]]; do probe=$(dirname "$probe"); done
  free=$(free_gb "$probe")
  local modetxt="full history"
  if ((SHALLOW)); then modetxt="shallow (depth $DEPTH)"; elif ((BLOBLESS)); then modetxt="blobless"; fi
  info "$total repositories into $DIR ($free GB free); mode: $modetxt"
  if ((DRY)); then
    printf '%s\n' "${SEL[@]}"
    return 0
  fi
  ((free >= MIN_FREE_GB)) || die "only $free GB free, --min-free-gb is $MIN_FREE_GB"
  mkdir -p -- "$DIR"
  touch -- "$DIR/$MARKER_CACHE"
  mkdir -p -- "$DIR/.logs"

  STATUS=$(mktemp)
  # shellcheck disable=SC2064
  trap "kill \$(jobs -p) 2>/dev/null || true; rm -f '$STATUS'; exit 130" INT TERM
  for line in "${SEL[@]}"; do
    i=$((i + 1))
    while (($(jobs -pr | wc -l) >= JOBS)); do wait -n || true; done
    clone_one "${line%% *}" "$i" "$total" "${line##* }" &
  done
  wait || true

  local ok skip fail
  ok=$(grep -c '^ok ' "$STATUS" || true)
  skip=$(grep -c '^skip ' "$STATUS" || true)
  fail=$(grep -c '^fail ' "$STATUS" || true)
  echo "cloned $ok, skipped $skip, failed $fail (of $total)"
  if ((fail)); then
    grep '^fail ' "$STATUS" | sed 's/^fail /  failed: /'
    echo "  git's messages are in $DIR/.logs/; run the same command again to retry" >&2
  fi
  rm -f "$STATUS"
  echo "disk used by the cache: $(du -sh -- "$DIR" 2>/dev/null | cut -f1)"
  ((fail == 0))
}

# ---------------------------------------------------------------------------------------
# open: launch Canopy and sample it
# ---------------------------------------------------------------------------------------
read_first() { # read_first FILE  -> first line, or empty
  local l=""
  [[ -r $1 ]] && IFS= read -r l <"$1" || true
  printf '%s' "$l"
}

write_meta() {
  local f=$1 npaths=$2 data=$3
  {
    echo "label=$LABEL"
    echo "date_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "start_epoch=$START_EPOCH"
    echo "repos=$npaths"
    echo "cache_dir=$DIR"
    echo "canopy_bin=$BIN"
    echo "canopy_version=$("$BIN" --version 2>/dev/null || echo unknown)"
    echo "canopy_log=$LOG_FILTER"
    echo "data_dir=$data"
    echo "git=$(git --version)"
    echo "kernel=$(uname -srm)"
    echo "os=$(. /etc/os-release 2>/dev/null && echo "${PRETTY_NAME:-unknown}")"
    echo "cpu=$(awk -F': ' '/^model name/ { print $2; exit }' /proc/cpuinfo)"
    echo "cpu_threads=$(nproc)"
    echo "mem_total_kb=$(awk '/^MemTotal:/ { print $2 }' /proc/meminfo)"
    echo "swap_total_kb=$(awk '/^SwapTotal:/ { print $2 }' /proc/meminfo)"
    echo "cache_fs=$(df -PT -- "$DIR" | awk 'NR == 2 { print $2 " on " $1 }')"
    echo "ulimit_nofile_soft=$(ulimit -Sn)"
    echo "inotify_max_user_watches=$(read_first /proc/sys/fs/inotify/max_user_watches)"
    echo "inotify_max_user_instances=$(read_first /proc/sys/fs/inotify/max_user_instances)"
    echo "session=${XDG_SESSION_TYPE:-unknown} ${XDG_CURRENT_DESKTOP:-}"
    if command -v lspci >/dev/null; then
      echo "gpu=$(lspci 2>/dev/null | grep -iE 'vga|3d|display' | head -n 2 | cut -d: -f3- | tr '\n' ';')"
    fi
    echo "note=record the disk type (NVMe, SATA SSD, HDD) of the cache folder by hand"
  } >"$f"
}

count_inotify() { # prints "<instances> <watches>" for pid $1
  local files n=0 inst=0 f
  # shellcheck disable=SC2012 # ls -l shows the link targets in one process
  files=$(ls -l "/proc/$1/fd" 2>/dev/null | awk -v p="$1" '/anon_inode:inotify/ { print "/proc/" p "/fdinfo/" $9 }') || true
  if [[ -n $files ]]; then
    for f in $files; do inst=$((inst + 1)); done
    # shellcheck disable=SC2086
    n=$(cat $files 2>/dev/null | grep -c '^inotify ' || true)
  fi
  echo "$inst $n"
}

thread_histogram() { # thread names and counts, for spotting leaks
  local pid=$1 elapsed=$2
  {
    echo "== t=${elapsed}s"
    cat /proc/"$pid"/task/*/comm 2>/dev/null | sort | uniq -c | sort -rn | head -n 12
  } >>"$RUN/threads.txt" || true
}

cmd_open() {
  [[ -d /proc/self ]] || die "'open' needs /proc (Linux)"
  cache_ready
  BIN=${BIN:-$ROOT/target/release/canopy}
  [[ -x $BIN ]] || die "no Canopy binary at $BIN (build: cargo build --release -p canopy-app; or use --bin)"
  [[ -n ${DISPLAY:-}${WAYLAND_DISPLAY:-} ]] || die "no display: start this from a desktop session"
  load_selection "$SUBSET"

  local -a paths=()
  local line repo missing=0
  for line in "${SEL[@]}"; do
    repo=${line%% *}
    if is_cached "$repo"; then paths+=("$DIR/$repo"); else missing=$((missing + 1)); fi
  done
  ((${#paths[@]})) || die "none of the selected repositories is cloned; run 'clone' first"
  ((missing == 0)) || info "warning: $missing selected repositories are not cloned and are left out"

  local watches instances
  watches=$(read_first /proc/sys/fs/inotify/max_user_watches)
  instances=$(read_first /proc/sys/fs/inotify/max_user_instances)
  # Canopy needs one instance for all its tabs; other programs of the user hold some too, so
  # warn below the kernel's default.
  local min_instances=128
  if [[ $instances =~ ^[0-9]+$ ]] && ((instances < min_instances)); then
    info "warning: fs.inotify.max_user_instances is $instances; Canopy needs one, other programs more. Raise it:"
    info "  sudo sysctl fs.inotify.max_user_instances=1024"
  fi
  if [[ $watches =~ ^[0-9]+$ ]] && ((watches < 1000000)); then
    info "warning: fs.inotify.max_user_watches is $watches; big trees need more. Raise it:"
    info "  sudo sysctl fs.inotify.max_user_watches=2097152"
  fi
  if (($(ulimit -Sn) < 4096)); then
    info "note: the open-files limit is $(ulimit -Sn); raise it in this shell with 'ulimit -n 65535' to avoid failures that are not Canopy's"
  fi

  START_EPOCH=$(date +%s)
  RUN=$RESULTS/run-$(date -u +%Y%m%dT%H%M%SZ)
  mkdir -p -- "$RUN"
  ln -sfn -- "$RUN" "$RESULTS/latest"
  # Canopy keeps its files in four XDG folders; the isolated run gives it all four under $data.
  local data state
  local -a folders=()
  if ((REAL_DATA)); then
    data=${XDG_DATA_HOME:-$HOME/.local/share}
    state=${XDG_STATE_HOME:-$HOME/.local/state}
  else
    data=$RESULTS/data
    state=$data/state
    folders=(XDG_DATA_HOME="$data" XDG_STATE_HOME="$state" XDG_CONFIG_HOME="$data/config" XDG_CACHE_HOME="$data/cache")
  fi
  mkdir -p -- "$data"
  printf '%s\n' "${paths[@]}" >"$RUN/repos.txt"
  write_meta "$RUN/meta.txt" "${#paths[@]}" "$data"
  : >"$RUN/crashes.txt"
  touch -- "$RUN/.start"

  info "run folder: $RUN"
  info "launching $BIN with ${#paths[@]} tabs (CANOPY_LOG=$LOG_FILTER)"
  env "${folders[@]}" CANOPY_LOG="$LOG_FILTER" "$BIN" "${paths[@]}" \
    >"$RUN/stdout.txt" 2>"$RUN/stderr.txt" &
  PID=$!
  echo "pid=$PID" >>"$RUN/meta.txt"
  trap 'kill -TERM "$PID" 2>/dev/null || true' INT TERM

  echo "ts,elapsed_s,rss_kb,rss_anon_kb,rss_file_kb,threads,fds,cpu_pct,children,inotify_fds,inotify_watches,mem_avail_kb" \
    >"$RUN/samples.csv"
  local hz
  hz=$(getconf CLK_TCK)
  local n=0 now prev_now prev_ticks=0 ticks cpu rss anon file thr fds kids ifd iw avail k v _ stat rest
  local -a st
  prev_now=$(date +%s.%N)
  local stopped=0
  while kill -0 "$PID" 2>/dev/null; do
    now=$(date +%s.%N)
    rss="" anon="" file="" thr=""
    {
      while read -r k v _; do
        case $k in
          VmRSS:) rss=$v ;;
          RssAnon:) anon=$v ;;
          RssFile:) file=$v ;;
          Threads:) thr=$v ;;
          *) ;;
        esac
      done <"/proc/$PID/status"
    } 2>/dev/null || true
    set -- /proc/"$PID"/fd/*
    fds=$#
    stat=$(cat "/proc/$PID/stat" 2>/dev/null) || stat=""
    cpu=""
    if [[ -n $stat ]]; then
      rest=${stat##*) }
      read -r -a st <<<"$rest"
      ticks=$((${st[11]:-0} + ${st[12]:-0}))
      cpu=$(awk -v a="$ticks" -v b="$prev_ticks" -v t="$now" -v u="$prev_now" -v hz="$hz" -v first="$((n == 0))" \
        'BEGIN { if (t > u && !first) printf "%.1f", (a - b) / hz / (t - u) * 100 }')
      prev_ticks=$ticks
    fi
    prev_now=$now
    kids=$(cat /proc/"$PID"/task/*/children 2>/dev/null | wc -w || true)
    ifd="" iw=""
    if ((n % 10 == 0)); then
      read -r ifd iw < <(count_inotify "$PID") || true
    fi
    if ((n % 300 == 30)); then thread_histogram "$PID" "$((${now%.*} - START_EPOCH))"; fi
    avail=$(awk '/^MemAvailable:/ { print $2 }' /proc/meminfo)
    if [[ -n $rss ]]; then
      printf '%s,%.1f,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n' "$now" \
        "$(awk -v a="$now" -v b="$START_EPOCH" 'BEGIN { print a - b }')" \
        "$rss" "$anon" "$file" "${thr:-}" "$fds" "$cpu" "$kids" "$ifd" "$iw" "$avail" >>"$RUN/samples.csv"
    fi
    n=$((n + 1))
    if [[ -n $DURATION && $stopped -eq 0 ]] && (($(date +%s) - START_EPOCH >= DURATION)); then
      info "duration reached: stopping Canopy"
      kill -TERM "$PID" 2>/dev/null || true
      stopped=$(date +%s)
    fi
    if ((stopped)) && (($(date +%s) - stopped > 30)); then
      info "Canopy did not exit within 30 s of SIGTERM: killing it (this is a finding)"
      kill -KILL "$PID" 2>/dev/null || true
    fi
    sleep "$SAMPLE_INTERVAL"
  done
  local code=0
  wait "$PID" || code=$?
  echo "exit_code=$code" >>"$RUN/meta.txt"

  # The app's own log (one file per run, named with the process id), and crash reports.
  local logfile
  logfile=$(find "$state/canopy/logs" -name "canopy-*-$PID.jsonl" 2>/dev/null | tail -n 1 || true)
  if [[ -n $logfile ]]; then
    cp -- "$logfile" "$RUN/canopy.jsonl"
  else
    info "warning: no log file found for pid $PID under $state/canopy/logs"
  fi
  find "$state"/canopy/crashes -name 'crash-*.txt' -newer "$RUN/.start" 2>/dev/null >"$RUN/crashes.txt" || true
  # Standard error carries the same events as text; keep only its tail if it grew large.
  if [[ -f $RUN/stderr.txt && $(stat -c %s "$RUN/stderr.txt") -gt 52428800 ]]; then
    tail -c 5242880 "$RUN/stderr.txt" >"$RUN/stderr.tail" && mv -- "$RUN/stderr.tail" "$RUN/stderr.txt"
  fi
  info "Canopy exited with status $code after $(($(date +%s) - START_EPOCH)) s; $(($(wc -l <"$RUN/samples.csv") - 1)) samples"
  if [[ -s $RUN/crashes.txt ]]; then info "CRASH REPORTS: $(wc -l <"$RUN/crashes.txt") (listed in crashes.txt)"; fi
  info "next: tools/stress.sh report --run $RUN"
}

# ---------------------------------------------------------------------------------------
# churn
# ---------------------------------------------------------------------------------------
now_ms() { date +%s%3N; }

in_progress() { # a merge, rebase, cherry-pick, revert or prepared conflict is under way
  local g=$1/.git
  [[ -e $g/MERGE_HEAD || -e $g/CHERRY_PICK_HEAD || -e $g/REVERT_HEAD || -d $g/rebase-merge ||
    -d $g/rebase-apply || -e $g/canopy-stress-conflict ]]
}

origin_branch() { read_first "$1/.git/canopy-stress-origin"; }

# Remember the branch a clone was on, and make the churn branches. Returns 1 if unusable.
churn_init() {
  local dir=$1 orig
  orig=$(origin_branch "$dir")
  if [[ -z $orig ]]; then
    orig=$(sgit "$dir" symbolic-ref -q --short HEAD 2>/dev/null) || return 1
    printf '%s\n' "$orig" >"$dir/.git/canopy-stress-origin"
  fi
  sgit "$dir" show-ref --verify --quiet "refs/heads/stress/a" ||
    sgit "$dir" branch -q stress/a "$orig" || return 1
  sgit "$dir" show-ref --verify --quiet "refs/heads/stress/b" ||
    sgit "$dir" branch -q stress/b "$orig" || return 1
}

LOGF=""
clog() { # clog REPO ACTION RC MS NOTE
  printf '%s %s %s %s %s %s\n' "$(now_ms)" "$1" "$2" "$3" "$4" "${5:-ok}" >>"$LOGF"
}

# Runs "$@" in repo $1 as action $2 and logs the outcome. Failures are classified.
act() {
  local repo=$1 name=$2 t0 rc=0 err note=ok
  shift 2
  t0=$(now_ms)
  err=$("$@" 2>&1 >/dev/null) || rc=$?
  if ((rc)); then
    case $err in
      *index.lock*) note=index-lock ;;
      *"would be overwritten"* | *"local changes"*) note=local-changes ;;
      *"REFUSED"*) note=refused ;;
      *) note=$(printf '%s' "$err" | head -n 1 | tr ' ' '_' | cut -c1-80) ;;
    esac
  fi
  clog "$repo" "$name" "$rc" "$(($(now_ms) - t0))" "${note:-failed}"
}

write_files() { # write_files DIR N SALT: create, edit and delete files under stress-churn/
  local dir=$1 count=$2 salt=$3 i sub f
  safe_repo "$dir" || {
    echo "REFUSED: $dir" >&2
    return 99
  }
  RANDOM=$((BASHPID + RANDOM)) # subshells would otherwise repeat the parent's sequence
  mkdir -p -- "$dir/stress-churn"
  for ((i = 0; i < count; i++)); do
    sub="$dir/stress-churn/d$((RANDOM % 4))"
    mkdir -p -- "$sub"
    f="$sub/f$(((RANDOM % 40) + salt % 7)).txt"
    case $((RANDOM % 6)) in
      0) rm -f -- "$f" ;;
      1) [[ -f $f ]] && mv -f -- "$f" "$f.moved" || printf 'x\n' >"$f" ;;
      *) seq 1 $((RANDOM % 80 + 5)) | sed "s/^/line $salt /" >>"$f" ;;
    esac
  done
}

commit_churn() { # commit_churn DIR N: commit what is in stress-churn/ (or nothing)
  local dir=$1 n=$2
  if [[ -d $dir/stress-churn ]]; then sgit "$dir" add -A -- stress-churn || return 1; fi
  sgit "$dir" commit -q --allow-empty --no-verify -m "stress: churn $n"
}

churn_repo() { # churn_repo REPO DIR END_EPOCH INDEX
  local repo=$1 dir=$2 end=$3 idx=$4 n=0 orig cur target choice r
  safe_repo "$dir" || return 0
  RANDOM=$((SEED * 1000 + idx))
  orig=$(origin_branch "$dir")
  while (($(date +%s) < end)); do
    n=$((n + 1))
    if in_progress "$dir"; then
      clog "$repo" skip 0 0 operation-in-progress
      sleep "$INTERVAL"
      continue
    fi
    r=$((RANDOM % 100))
    if ((r < 35)); then
      act "$repo" files write_files "$dir" "$((RANDOM % FILES + 1))" "$n"
    elif ((r < 55)); then
      act "$repo" commit commit_churn "$dir" "$n"
    elif ((r < 75)); then
      cur=$(sgit "$dir" symbolic-ref -q --short HEAD 2>/dev/null || true)
      case $((RANDOM % 3)) in 0) target=stress/a ;; 1) target=stress/b ;; *) target=$orig ;; esac
      [[ $target == "$cur" ]] && target=stress/b
      [[ $target == "$cur" ]] && target=stress/a
      act "$repo" checkout sgit "$dir" checkout -q "$target"
    elif ((r < 85)); then
      choice=$((RANDOM % 3))
      case $choice in
        0) act "$repo" tag sgit "$dir" tag -f "stress/t$((n % 25))" ;;
        1) act "$repo" branch sgit "$dir" branch -f "stress/n$((n % 25))" ;;
        *) act "$repo" ref-delete sgit "$dir" update-ref -d "refs/tags/stress/t$(((n + 12) % 25))" ;;
      esac
    elif ((r < 95)); then
      act "$repo" modify-tracked modify_tracked "$dir" 1
    elif ((DEEP)) && [[ $(mode_of "$dir") == full ]]; then
      act "$repo" deep-checkout deep_checkout "$dir" "$orig" "$((RANDOM % 200 + 20))"
    else
      act "$repo" status sgit "$dir" status --porcelain
    fi
    sleep "$(awk -v i="$INTERVAL" -v r="$RANDOM" 'BEGIN { printf "%.2f", i * (0.5 + r / 32767) }')"
  done
}

deep_checkout() { # deep_checkout DIR BRANCH N: detach N commits back, if the history is that long
  local c
  c=$(sgit "$1" rev-parse --verify -q "$2~$3^{commit}") || {
    echo "history-too-short" >&2
    return 1
  }
  sgit "$1" checkout -q --detach "$c"
}

mode_of() { sed -n 's/^mode=//p' "$1/.git/$MARKER_REPO" | head -n 1; }

# Appends a line to N random tracked regular files of repo $1 (cache clones only).
modify_tracked() {
  local dir=$1 count=$2 f
  safe_repo "$dir" || {
    echo "REFUSED: $dir" >&2
    return 99
  }
  RANDOM=$((BASHPID + RANDOM))
  while IFS= read -r -d '' f; do
    [[ -f $dir/$f && ! -L $dir/$f ]] || continue
    printf '\n// stress %s\n' "$RANDOM" >>"$dir/$f"
  done < <(git -C "$dir" ls-files -z 2>/dev/null | shuf -z -n "$count" --random-source=<(yes "$RANDOM"))
}

conflict_setup() { # conflict_setup REPO DIR
  local repo=$1 dir=$2 orig side
  safe_repo "$dir" || return 0
  churn_init "$dir" || { echo "  skip $repo (no branch checked out)"; return 0; }
  orig=$(origin_branch "$dir")
  rm -f -- "$dir/.git/canopy-stress-conflict"
  # Start from a clean tree: drops churn leftovers (this is a stress clone, checked above).
  sgit "$dir" clean -fdq -- stress-churn stress-conflict.txt || return 1
  sgit "$dir" checkout -q -f "$orig" || return 1
  sgit "$dir" branch -q -f stress/conflict-a "$orig" || return 1
  sgit "$dir" branch -q -f stress/conflict-b "$orig" || return 1
  for side in a b; do
    sgit "$dir" checkout -q "stress/conflict-$side" || return 1
    seq 1 200 | sed "s/^/side $side line /" >"$dir/stress-conflict.txt"
    if [[ -f $dir/README.md ]]; then
      { printf 'edited on side %s\n' "$side"; cat -- "$dir/README.md"; } >"$dir/README.md.new" &&
        mv -f -- "$dir/README.md.new" "$dir/README.md"
    fi
    sgit "$dir" add -A -- stress-conflict.txt README.md 2>/dev/null ||
      sgit "$dir" add -A -- stress-conflict.txt || return 1
    sgit "$dir" commit -q --no-verify -m "stress: conflict side $side" || return 1
  done
  sgit "$dir" checkout -q stress/conflict-a || return 1
  touch -- "$dir/.git/canopy-stress-conflict"
  if ((LEAVE_MERGING)); then
    sgit "$dir" merge --no-edit stress/conflict-b >/dev/null 2>&1 || true
    echo "  $repo: merge left in progress"
  else
    echo "  $repo: on stress/conflict-a; merge stress/conflict-b into it in Canopy"
  fi
}

restore_repo() { # restore_repo REPO DIR
  local repo=$1 dir=$2 orig ref d
  safe_repo "$dir" || return 0
  orig=$(origin_branch "$dir")
  [[ -n $orig ]] || return 0
  [[ -e $dir/.git/MERGE_HEAD ]] && sgit "$dir" merge --abort >/dev/null 2>&1 || true
  for d in rebase-merge rebase-apply; do
    [[ -d $dir/.git/$d ]] && sgit "$dir" rebase --abort >/dev/null 2>&1 || true
  done
  sgit "$dir" checkout -q -f "$orig" 2>/dev/null || sgit "$dir" checkout -q -f "refs/heads/$orig"
  sgit "$dir" clean -fdq -- stress-churn stress-conflict.txt 2>/dev/null || true
  while IFS= read -r ref; do
    [[ $ref == refs/heads/stress/* || $ref == refs/tags/stress/* ]] || continue
    sgit "$dir" update-ref -d "$ref"
  done < <(sgit "$dir" for-each-ref --format='%(refname)' refs/heads/stress refs/tags/stress)
  rm -f -- "$dir/.git/canopy-stress-origin" "$dir/.git/canopy-stress-conflict"
  echo "  restored $repo ($orig)"
}

cmd_churn() {
  command -v shuf >/dev/null || die "shuf (GNU coreutils) is required"
  cache_ready
  local -a targets=()
  local line repo

  if ((RESTORE)); then
    # Every cached clone that carries the origin marker, not only a selection.
    load_selection 0
    for line in "${SEL[@]}"; do
      repo=${line%% *}
      is_cached "$repo" && [[ -f $DIR/$repo/.git/canopy-stress-origin ]] && targets+=("$repo")
    done
    info "restoring ${#targets[@]} clones"
    for repo in ${targets[@]+"${targets[@]}"}; do restore_repo "$repo" "$DIR/$repo"; done
    return 0
  fi

  # Evenly spaced over the cached repositories that are usable, so every size class takes part.
  load_selection 0
  local -a cached=()
  for line in "${SEL[@]}"; do
    repo=${line%% *}
    is_cached "$repo" && safe_repo "$DIR/$repo" && cached+=("$repo")
  done
  ((${#cached[@]})) || die "no cached clones; run 'clone' first"
  local total=${#cached[@]} want=$REPOS i
  ((want > total)) && want=$total
  for ((i = 0; i < want; i++)); do targets+=("${cached[$((i * total / want))]}"); done

  LOGF=${CHURN_LOG:-}
  if [[ -z $LOGF ]]; then
    if [[ -d $RESULTS/latest ]]; then LOGF=$RESULTS/latest/churn-$(date -u +%H%M%S).log; else LOGF=$RESULTS/churn-$(date -u +%Y%m%dT%H%M%SZ).log; fi
  fi
  mkdir -p -- "$(dirname "$LOGF")"
  : >"$LOGF"

  if ((CONFLICT)); then
    info "preparing conflicts in ${#targets[@]} clones"
    for repo in "${targets[@]}"; do
      conflict_setup "$repo" "$DIR/$repo" || echo "  FAILED $repo (see git's message above)"
    done
    return 0
  fi
  if ((MODIFY_MANY)); then
    info "modifying $MODIFY_MANY tracked files in each of ${#targets[@]} clones"
    for repo in "${targets[@]}"; do
      churn_init "$DIR/$repo" || { echo "  skip $repo"; continue; }
      act "$repo" modify-many modify_tracked "$DIR/$repo" "$MODIFY_MANY"
      echo "  $repo done"
    done
    return 0
  fi

  local end=$(($(date +%s) + ${DURATION:-300})) idx=0 started=0
  info "churning ${#targets[@]} clones for ${DURATION:-300} s; action log: $LOGF"
  trap 'kill $(jobs -p) 2>/dev/null || true' INT TERM
  for repo in "${targets[@]}"; do
    idx=$((idx + 1))
    if ! churn_init "$DIR/$repo"; then
      clog "$repo" init 1 0 no-branch-checked-out
      continue
    fi
    churn_repo "$repo" "$DIR/$repo" "$end" "$idx" &
    started=$((started + 1))
  done
  wait || true
  info "churn finished ($started clones)"
  awk '
    { n[$3]++; if ($4 != 0 && $3 != "skip") f[$6]++ ; if ($4 != 0 && $3 != "skip") bad++ }
    END {
      printf "actions:"; for (a in n) printf " %s=%d", a, n[a]; printf "\n"
      printf "failed actions: %d\n", bad + 0
      for (k in f) printf "  %s: %d\n", k, f[k]
    }' "$LOGF" >&2
  info "failures marked index-lock mean git could not take the lock because another process (maybe Canopy) held it; check the app log"
  info "restore the clones with: tools/stress.sh churn --restore"
}

# ---------------------------------------------------------------------------------------
# report
# ---------------------------------------------------------------------------------------
cmd_report() {
  command -v python3 >/dev/null || die "python3 is required"
  local -a args=()
  if [[ -z $REPORT_LOG && -z $REPORT_CSV ]]; then
    RUN_DIR=${RUN_DIR:-$RESULTS/latest}
    [[ -d $RUN_DIR ]] || die "no run folder at $RUN_DIR (run 'open' first, or pass --run/--log/--csv)"
    args+=(--run "$RUN_DIR")
  else
    [[ -z $REPORT_LOG ]] || args+=(--log "$REPORT_LOG")
    [[ -z $REPORT_CSV ]] || args+=(--csv "$REPORT_CSV")
    [[ -z $REPORT_CHURN ]] || args+=(--churn "$REPORT_CHURN")
  fi
  exec python3 "$SCRIPT_DIR/stress_report.py" "${args[@]}" ${PASSTHROUGH[@]+"${PASSTHROUGH[@]}"}
}

case $CMD in
  clone) cmd_clone ;;
  open) cmd_open ;;
  churn) cmd_churn ;;
  report) cmd_report ;;
  *) die "unknown command '$CMD' (clone, open, churn, report; see --help)" ;;
esac
