#!/usr/bin/env bash
# Usage: tools/mkfixture.sh [--force] <dir> [kind]
#
# Creates a deterministic demo repository at <dir> with plain `git`. Identities and dates are
# fixed and the user's global and system git config are ignored, so the same kind always produces
# the same commit IDs. Use it for screenshots, manual testing and anything destructive; never
# point Canopy at a real repository for that.
#
# Kinds:
#   showcase  (default) ~40 commits of a small Rust/web project: merged and open feature branches,
#             a fast-forwarded fix, a release branch, annotated tags v0.1.0 v0.2.0 v1.0.0, a bare
#             "remote" at <dir>.remote.git added as origin (main ahead of origin/main by 2,
#             feature/dark-mode behind its upstream by 1), one stash entry, and a dirty working
#             tree: a modified file with two hunks, a staged change, an untracked file and a
#             deleted file.
#   conflict  a repository stopped mid-merge with one conflicted file.
#   linear    100 linear commits.
#   big       BIG_COMMITS (default 5000) mainline commits written with `git fast-import`, with a
#             three-commit topic branch merged back every 50 commits; for performance checks.
#
# --force replaces <dir> (and <dir>.remote.git) only if they were created by this script.
set -euo pipefail

usage() { awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"; }

force=0
if [[ ${1:-} == "--force" ]]; then force=1; shift; fi
if [[ $# -lt 1 || ${1:-} == "-h" || ${1:-} == "--help" ]]; then usage; exit 2; fi

DIR=$1
KIND=${2:-showcase}
REMOTE="$DIR.remote.git"
MARKER=canopy-fixture

case $KIND in showcase | conflict | linear | big) ;; *) echo "unknown kind: $KIND" >&2; usage; exit 2 ;; esac

# Isolate from the user's environment: no global/system config, hooks, signing or templates.
export GIT_CONFIG_NOSYSTEM=1
export GIT_CONFIG_GLOBAL=/dev/null
export LC_ALL=C
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_TEMPLATE_DIR

remove_fixture() {
  local path=$1 marker=$2
  [[ -e $path ]] || return 0
  if [[ $force -eq 1 && -f $marker ]]; then
    rm -rf -- "$path"
  else
    echo "error: $path already exists (use --force to replace a fixture made by this script)" >&2
    exit 1
  fi
}
remove_fixture "$DIR" "$DIR/.git/$MARKER"
if [[ $KIND == showcase ]]; then remove_fixture "$REMOTE" "$REMOTE/$MARKER"; fi

mkdir -p "$DIR"
REPO=$(cd "$DIR" && pwd)

g() {
  git -C "$REPO" -c commit.gpgsign=false -c tag.gpgsign=false -c core.autocrlf=false \
    -c advice.detachedHead=false -c init.defaultBranch=main "$@"
}

AUTHORS=(
  "Maya Chen|maya.chen@example.com|+0200"
  "Jonas Weber|jonas.weber@example.org|+0200"
  "Priya Raman|priya.raman@example.net|+0530"
  "Sam Okafor|sam.okafor@example.com|+0100"
  "Lucía Ortega|lucia.ortega@example.org|-0300"
)
MAYA=0 JONAS=1 PRIYA=2 SAM=3 LUCIA=4

clock=1785747600 # 2026-08-03T09:00:00Z
step=0
by() {
  local name email tz
  IFS='|' read -r name email tz <<<"${AUTHORS[$1]}"
  step=$((step + 1))
  clock=$((clock + 3600 * (1 + (step * 7) % 17) + (step * 131) % 3600))
  export GIT_AUTHOR_NAME=$name GIT_AUTHOR_EMAIL=$email GIT_AUTHOR_DATE="$clock $tz"
  export GIT_COMMITTER_NAME=$name GIT_COMMITTER_EMAIL=$email GIT_COMMITTER_DATE="$clock $tz"
}

put() { mkdir -p "$(dirname "$REPO/$1")"; cat >"$REPO/$1"; }
# rewrite <file> <sed arguments...>: edit a file in place without GNU/BSD `sed -i` differences.
rewrite() {
  local out
  out=$(sed "${@:2}" "$REPO/$1")
  printf '%s\n' "$out" >"$REPO/$1"
}
append() { printf '%s\n' "$2" >>"$REPO/$1"; }
commit() {
  by "$1"
  g add -A
  if [[ $# -ge 3 ]]; then g commit -q -m "$2" -m "$3"; else g commit -q -m "$2"; fi
}
merge() { by "$1"; g merge -q --no-ff --no-edit "$2"; }
tag() { by "$1"; g tag -a "$2" -m "$3"; }

init_repo() {
  g init -q -b main
  touch "$REPO/.git/$MARKER"
}

main_rs() { # main_rs <variant>
  local listen='"127.0.0.1:8080"' extra='' limiter='' mods='' shutdown='let _ = tokio::signal::ctrl_c().await;'
  if [[ $1 -ge 2 ]]; then limiter='    let limiter = auth::RateLimiter::new(5, Duration::from_secs(60));'; fi
  if [[ $1 -ge 3 ]]; then listen='config.listen_addr()'; extra='    tracing::info!(%addr, "tidepool ready");'; fi
  if [[ $1 -ge 4 ]]; then
    mods=$'\nmod metrics;'
    shutdown='tokio::signal::ctrl_c().await.expect("install Ctrl+C handler");'
  fi
  put src/main.rs <<EOF
//! Tidepool: a small, fast pastebin with syntax highlighting.

mod auth;
mod config;$mods
mod routes;
mod storage;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use tokio::net::TcpListener;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<storage::Store>,
    pub sessions: Arc<auth::Sessions>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("tidepool=info").init();
    let config = config::Config::from_env()?;

    let store = Arc::new(storage::Store::open(&config.data_dir).await?);
    let sessions = Arc::new(auth::Sessions::new(Duration::from_secs(60 * 60 * 24)));
$limiter
    let state = AppState { store, sessions };

    let app: Router = routes::router(state.clone());

    let addr: SocketAddr = $listen.parse()?;
    let listener = TcpListener::bind(addr).await?;
$extra
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    $shutdown
    tracing::info!("shutting down");
}
EOF
}

make_showcase() {
  init_repo

  put README.md <<'EOF'
# Tidepool

A small, fast pastebin with syntax highlighting.

## Running

    cargo run

Then open http://127.0.0.1:8080.
EOF
  put .gitignore <<'EOF'
/target
/data
EOF
  put Cargo.toml <<'EOF'
[package]
name = "tidepool"
version = "0.1.0"
edition = "2021"

[dependencies]
anyhow = "1"
axum = "0.7"
serde = { version = "1.0.200", features = ["derive"] }
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
tracing-subscriber = "0.3"
EOF
  commit $MAYA "chore: initial commit"

  main_rs 1
  put src/config.rs <<'EOF'
pub struct Config {
    pub data_dir: std::path::PathBuf,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let data_dir = std::env::var_os("TIDEPOOL_DATA").unwrap_or_else(|| "data".into());
        Ok(Self { data_dir: data_dir.into() })
    }
}
EOF
  commit $MAYA "feat: add HTTP server skeleton"

  put src/routes.rs <<'EOF'
use axum::{routing::get, Router};

use crate::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .with_state(state)
}

async fn index() -> &'static str {
    "tidepool"
}

async fn health() -> &'static str {
    "ok"
}
EOF
  commit $MAYA "feat(routes): add health and index routes"

  append README.md ""
  append README.md "## Development"
  append README.md ""
  append README.md "Set TIDEPOOL_DATA to choose where pastes are stored (default: ./data)."
  commit $SAM "docs: describe local development setup"

  put web/index.html <<'EOF'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>Tidepool</title>
  <link rel="stylesheet" href="app.css">
</head>
<body>
  <main id="app"></main>
  <script src="legacy.js"></script>
</body>
</html>
EOF
  put web/app.css <<'EOF'
:root {
  --bg: #ffffff;
  --text: #1b1f24;
  --accent: #2f6feb;
}

body {
  margin: 0;
  font: 15px/1.5 system-ui, sans-serif;
  background: var(--bg);
  color: var(--text);
}

header {
  padding: 12px 16px;
  border-bottom: 1px solid #e5e7eb;
}
EOF
  put web/legacy.js <<'EOF'
// Polyfills for browsers without fetch(). Remove once usage drops below 1%.
window.fetch = window.fetch || function () { throw new Error("fetch unsupported"); };
EOF
  commit $PRIYA "build: add web assets pipeline"

  put tests/routes.rs <<'EOF'
#[tokio::test]
async fn health_returns_ok() {
    let body = tidepool::test_client().get("/health").await.text().await;
    assert_eq!(body, "ok");
}
EOF
  commit $SAM "test: cover route handlers"
  tag $SAM v0.1.0 "Tidepool 0.1.0"

  # feature/auth, merged with --no-ff while main moves on.
  g switch -q -c feature/auth
  put src/auth.rs <<'EOF'
use std::time::Duration;

pub struct Sessions {
    ttl: Duration,
}

impl Sessions {
    pub fn new(ttl: Duration) -> Self {
        Self { ttl }
    }
}
EOF
  commit $JONAS "feat(auth): add session token type"
  append Cargo.toml 'argon2 = "0.5"'
  append src/auth.rs ""
  append src/auth.rs "pub fn hash_password(password: &str) -> anyhow::Result<String> {"
  append src/auth.rs "    crate::crypto::argon2_hash(password)"
  append src/auth.rs "}"
  commit $JONAS "feat(auth): hash passwords with argon2"
  put tests/auth.rs <<'EOF'
#[test]
fn login_round_trip() {
    let hash = tidepool::auth::hash_password("hunter2").unwrap();
    assert!(tidepool::auth::verify_password("hunter2", &hash));
}
EOF
  commit $JONAS "test(auth): add login round-trip test"

  g switch -q main
  append src/routes.rs ""
  append src/routes.rs "async fn not_found() -> axum::http::StatusCode {"
  append src/routes.rs "    axum::http::StatusCode::NOT_FOUND"
  append src/routes.rs "}"
  commit $MAYA "fix(routes): return 404 for unknown paths"
  put .github/workflows/ci.yml <<'EOF'
name: CI
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo test --locked
EOF
  commit $SAM "ci: run tests on pull requests"
  merge $MAYA feature/auth
  append README.md ""
  append README.md "Sessions expire after 24 hours of inactivity."
  commit $JONAS "docs(auth): document session lifetime"

  # feature/graph, merged with --no-ff.
  g switch -q -c feature/graph
  put src/graph.rs <<'EOF'
/// One row of the revision graph shown on the history page.
pub struct Row {
    pub lane: usize,
    pub parents: Vec<usize>,
}
EOF
  commit $PRIYA "feat(graph): add revision graph data model"
  put web/graph.js <<'EOF'
export function renderLanes(svg, rows) {
  for (const row of rows) {
    svg.append(circle(row.lane * 16 + 8, row.index * 24 + 12));
  }
}
EOF
  commit $PRIYA "feat(graph): render lanes as SVG"
  g switch -q main
  rewrite Cargo.toml -e 's/serde = { version = "1.0.200"/serde = { version = "1.0.210"/'
  commit $SAM "chore(deps): bump serde to 1.0.210"
  g switch -q feature/graph
  append src/graph.rs ""
  append src/graph.rs "/// Lane assignments are cached per revision range."
  append src/graph.rs "pub type LaneCache = std::collections::HashMap<String, Vec<Row>>;"
  commit $PRIYA "perf(graph): cache lane assignment"
  put tests/graph.rs <<'EOF'
#[test]
fn merge_rows_keep_first_parent_lane() {
    let rows = tidepool::graph::layout(&tidepool::graph::fixtures::merge());
    assert_eq!(rows[0].lane, rows[1].lane);
}
EOF
  commit $PRIYA "test(graph): cover merge rows"
  g switch -q main
  merge $PRIYA feature/graph
  tag $MAYA v0.2.0 "Tidepool 0.2.0"

  # fix/typo, fast-forwarded.
  g switch -q -c fix/typo
  rewrite README.md -e 's/A small, fast pastebin/A small, fast paste bin/'
  commit $LUCIA "docs: fix typo in README"
  g switch -q main
  g merge -q --ff-only fix/typo

  put web/settings.html <<'EOF'
<!doctype html>
<title>Settings - Tidepool</title>
<form method="post" action="/settings">
  <label>Default expiry <select name="expiry"><option>1 day</option><option>1 week</option></select></label>
  <button>Save</button>
</form>
EOF
  commit $MAYA "feat(ui): add settings page"
  main_rs 2
  commit $JONAS "refactor: build rate limiter at startup"
  append src/auth.rs ""
  append src/auth.rs "pub fn is_expired(issued_at: std::time::Instant, ttl: Duration) -> bool {"
  append src/auth.rs "    issued_at.elapsed() > ttl"
  append src/auth.rs "}"
  commit $JONAS "fix(auth): reject expired sessions" "Sessions were checked for existence only, so a stolen token stayed valid forever."

  # release/1.0, tagged and merged back.
  g switch -q -c release/1.0
  rewrite Cargo.toml -e 's/^version = "0.1.0"/version = "1.0.0"/'
  commit $SAM "chore(release): bump version to 1.0.0"
  put CHANGELOG.md <<'EOF'
# Changelog

## 1.0.0

- Accounts with password login and expiring sessions.
- Revision graph on the history page.
- Settings page with default expiry.
EOF
  commit $SAM "docs(changelog): add 1.0.0 notes"
  tag $SAM v1.0.0 "Tidepool 1.0.0"
  g switch -q main
  merge $SAM release/1.0

  # feature/dark-mode, left open.
  g switch -q -c feature/dark-mode
  put web/tokens.css <<'EOF'
:root {
  --surface: #ffffff;
  --surface-muted: #f3f4f6;
  --border: #e5e7eb;
}
EOF
  commit $LUCIA "feat(ui): add colour tokens"
  put web/dark.css <<'EOF'
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #0d1117;
    --text: #e6edf3;
    --surface: #161b22;
    --border: #30363d;
  }
}
EOF
  commit $LUCIA "feat(ui): add dark theme stylesheet"
  append web/index.html "<!-- theme toggle lives in the header partial -->"
  commit $LUCIA "feat(ui): add theme toggle to header"

  g switch -q main
  append src/routes.rs ""
  append src/routes.rs "pub const PAGE_SIZE: usize = 50;"
  commit $PRIYA "feat(api): paginate list endpoints" "Lists return at most PAGE_SIZE items and a next-page cursor."
  put web/search.js <<'EOF'
let timer;
export function onSearchInput(event, run) {
  clearTimeout(timer);
  timer = setTimeout(() => run(event.target.value), 200);
}
EOF
  commit $MAYA "fix(web): debounce search input"
  rewrite Cargo.toml -e 's/^tokio = { version = "1"/tokio = { version = "1.40"/'
  commit $SAM "chore(deps): update tokio to 1.40"
  main_rs 3
  commit $JONAS "feat(api): rate-limit login attempts"

  # The "remote": a bare repository holding main, release/1.0 and feature/dark-mode.
  git init -q --bare -b main "$REMOTE"
  touch "$REMOTE/$MARKER"
  local remote_abs
  remote_abs=$(cd "$REMOTE" && pwd)
  g remote add origin "$remote_abs"
  g push -q -u origin main release/1.0 feature/dark-mode
  g push -q origin --tags
  g remote set-head origin main

  # feature/dark-mode: upstream has one commit the local branch does not.
  g switch -q feature/dark-mode
  append web/dark.css "/* muted text meets WCAG AA on --surface */"
  commit $LUCIA "fix(ui): improve contrast of muted text"
  g push -q origin feature/dark-mode
  g reset -q --hard HEAD~1

  # main: two local commits not yet pushed.
  g switch -q main
  append src/routes.rs ""
  append src/routes.rs "pub const STATIC_CACHE: &str = \"public, max-age=3600\";"
  commit $PRIYA "feat(api): add ETag headers to static assets"
  put docs/api.md <<'EOF'
# API

| Method | Path | Description |
|---|---|---|
| GET | /health | Liveness probe |
| POST | /pastes | Create a paste |
| GET | /pastes/:id | Fetch a paste |
EOF
  commit $MAYA "docs: add API reference"

  # A stash entry on a clean tree.
  append web/app.css ""
  append web/app.css "header { padding: 16px 24px; }"
  by $MAYA
  g stash push -q -m "wip: tweak header spacing"

  # Dirty working tree.
  main_rs 4
  append README.md ""
  append README.md "See docs/api.md for the HTTP API."
  g add README.md
  put src/metrics.rs <<'EOF'
//! Prometheus metrics. Not wired up yet.
pub fn install() {}
EOF
  rm "$REPO/web/legacy.js"
}

make_conflict() {
  init_repo
  put src/retry.rs <<'EOF'
pub struct RetryPolicy {
    pub attempts: u32,
    pub backoff_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { attempts: 3, backoff_ms: 200 }
    }
}
EOF
  put README.md <<'EOF'
# Retry

Retry policies for flaky network calls.
EOF
  commit $MAYA "feat: add retry policy"
  g switch -q -c feature/backoff
  rewrite src/retry.rs -e 's/attempts: 3, backoff_ms: 200/attempts: 5, backoff_ms: 500/'
  append README.md "Backoff doubles after every failed attempt."
  commit $JONAS "feat(retry): use exponential backoff defaults"
  g switch -q main
  rewrite src/retry.rs -e 's/attempts: 3, backoff_ms: 200/attempts: 4, backoff_ms: 250/'
  commit $PRIYA "fix(retry): tune defaults for mobile clients"
  by $MAYA
  if g merge -q --no-edit feature/backoff >/dev/null 2>&1; then
    echo "error: expected a merge conflict" >&2
    exit 1
  fi
}

make_linear() {
  init_repo
  local types=(feat fix docs refactor test perf chore build)
  local areas=(parser cli config cache api docs ui storage)
  local i
  put CHANGES.txt </dev/null
  for i in $(seq 1 100); do
    append CHANGES.txt "change $i"
    commit $((i % 5)) "${types[i % 8]}(${areas[(i * 3) % 8]}): change number $i"
  done
}

make_big() {
  init_repo
  local total=${BIG_COMMITS:-5000}
  local mark=0 main_tip=0 i j topic_tip msg content ts
  emit_commit() { # emit_commit <ref> <parent-mark> <merge-mark|0> <author-index> <message> <path>
    local ref=$1 parent=$2 other=$3 who=$4 message=$5 path=$6 name email tz
    IFS='|' read -r name email tz <<<"${AUTHORS[$who]}"
    mark=$((mark + 1))
    ts=$((1777885200 + mark * 1800))
    content="revision $mark"
    printf 'commit %s\nmark :%d\nauthor %s <%s> %d +0000\ncommitter %s <%s> %d +0000\n' \
      "$ref" "$mark" "$name" "$email" "$ts" "$name" "$email" "$ts"
    printf 'data %d\n%s\n' "${#message}" "$message"
    if [[ $parent -ne 0 ]]; then printf 'from :%d\n' "$parent"; fi
    if [[ $other -ne 0 ]]; then printf 'merge :%d\n' "$other"; fi
    printf 'M 100644 inline %s\ndata %d\n%s\n\n' "$path" "${#content}" "$content"
  }
  {
    for ((i = 1; i <= total; i++)); do
      if ((i % 50 == 0)); then
        topic_tip=$main_tip
        for ((j = 1; j <= 3; j++)); do
          emit_commit "refs/heads/topic/$i" "$topic_tip" 0 $(((i + j) % 5)) \
            "feat(topic-$i): step $j" "src/topic_$((i % 40)).rs"
          topic_tip=$mark
        done
        emit_commit refs/heads/main "$main_tip" "$topic_tip" $((i % 5)) \
          "Merge branch 'topic/$i'" "src/merge.rs"
      else
        emit_commit refs/heads/main "$main_tip" 0 $((i % 5)) \
          "chore(src): revision $i" "src/mod_$((i % 20)).rs"
      fi
      main_tip=$mark
      if ((i % 500 == 0)); then printf 'reset refs/tags/v0.%d.0\nfrom :%d\n\n' $((i / 500)) "$mark"; fi
    done
  } | g fast-import --quiet
  g reset -q --hard main
}

"make_$KIND"
echo "$REPO ($KIND)"
