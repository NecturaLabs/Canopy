#!/bin/sh
# Usage: packaging/test-install.sh
#
# End-to-end test of packaging/install.sh on Linux against a fake update feed: builds fake release
# archives (a dummy `canopy` script), a manifest and SHA256SUMS, serves them with
# `python3 -m http.server` on 127.0.0.1, and runs the installer with a temporary HOME through its
# test-only CANOPY_INSTALL_BASE_URL. Never touches the real HOME. Needs python3, curl and tar.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd -P)
installer=$root/packaging/install.sh
[ "$(uname -s)" = Linux ] || {
    echo "test-install.sh runs on Linux only" >&2
    exit 1
}
case $(uname -m) in
    x86_64 | amd64) target=x86_64-unknown-linux-gnu key=linux-x86_64 other=linux-aarch64 ;;
    aarch64 | arm64) target=aarch64-unknown-linux-gnu key=linux-aarch64 other=linux-x86_64 ;;
    *)
        echo "unsupported test machine: $(uname -m)" >&2
        exit 1
        ;;
esac
# The shell that runs the installer: dash when present (the strictest common POSIX shell);
# CANOPY_TEST_SHELL picks another, e.g. bash or "busybox sh".
SH="sh"
if command -v dash >/dev/null 2>&1; then SH=dash; fi
SH=${CANOPY_TEST_SHELL:-$SH}

tmp=$(mktemp -d "${TMPDIR:-/tmp}/canopy-install-test.XXXXXX")
server_pid=
cleanup() {
    if [ -n "$server_pid" ]; then kill "$server_pid" 2>/dev/null || true; fi
    rm -rf -- "$tmp"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

home=$tmp/home
srv=$tmp/srv
mkdir -p "$home" "$srv/canopy" "$srv/releases/download"
cp "$installer" "$srv/canopy/install.sh"

failures=0
pass() { printf 'ok   %s\n' "$*"; }
fail() {
    printf 'FAIL %s\n' "$*"
    failures=$((failures + 1))
}
check() {
    desc=$1
    shift
    if "$@"; then pass "$desc"; else fail "$desc"; fi
}

sha() { sha256sum -- "$1" | awk '{print $1}'; }

# Builds release $1 into the fake feed.
make_release() {
    v=$1
    name=canopy-$v-$target
    d=$tmp/build/$name
    mkdir -p "$d/legal"
    printf '#!/bin/sh\necho "canopy %s (fake)"\n' "$v" >"$d/canopy"
    chmod 755 "$d/canopy"
    echo "Canopy" >"$d/README.md"
    echo "Proprietary" >"$d/LICENSE"
    echo "notices" >"$d/legal/third-party-notices.txt"
    cp "$root/packaging/linux/io.necturalabs.Canopy.desktop" "$root/packaging/linux/io.necturalabs.Canopy.metainfo.xml" "$d/"
    cp "$root/assets/icon.svg" "$d/io.necturalabs.Canopy.svg"
    cp "$root/assets/icon-256.png" "$d/io.necturalabs.Canopy.png"
    rel=$srv/releases/download/canopy-v$v
    mkdir -p "$rel"
    tar -czf "$rel/$name.tar.gz" -C "$tmp/build" "$name"
    (cd "$rel" && sha256sum -- *.tar.gz >SHA256SUMS)
}

# Writes channel manifest $1 for version $2; $3 overrides the checksum.
publish() {
    ch=$1 v=$2
    name=canopy-$v-$target
    digest=${3:-$(sha "$srv/releases/download/canopy-v$v/$name.tar.gz")}
    cat >"$srv/canopy/$ch.json" <<EOF
{
  "version": "$v",
  "notes": "Notes with \"quotes\", braces { } and a fake \"platforms\": {\"$key\": {\"url\": \"https://evil.invalid/x\"}}\\n- line",
  "pub_date": "2026-10-01T00:00:00Z",
  "release_page": "$base/releases/tag/canopy-v$v",
  "platforms": {
    "$key": {
      "url": "$base/releases/download/canopy-v$v/$name.tar.gz",
      "sha256": "$digest"
    }
  }
}
EOF
}

# Runs the installer with a clean environment and the temporary HOME.
run_path=/usr/local/bin:/usr/bin:/bin
run() {
    env -i HOME="$home" PATH="$run_path" CANOPY_INSTALL_BASE_URL="$base" \
        "$SH" "$installer" "$@" >"$tmp/out" 2>&1
}
run_status() {
    set +e
    run "$@"
    status=$?
    set -e
}
out_has() { grep -qF -- "$1" "$tmp/out"; }
# Shows the installer output of the step above, indented, so the log reads like a session.
show() { sed 's/^/  | /' "$tmp/out"; }

# Serve the fake feed. `-u`: the port line must reach the log unbuffered.
python3 -u -m http.server --bind 127.0.0.1 0 --directory "$srv" >"$tmp/server.log" 2>&1 &
server_pid=$!
port=
i=0
while [ -z "$port" ] && [ $i -lt 100 ]; do
    port=$(sed -n 's/.*port \([0-9][0-9]*\).*/\1/p' "$tmp/server.log" | head -n 1)
    [ -n "$port" ] || sleep 0.1
    i=$((i + 1))
done
[ -n "$port" ] || {
    cat "$tmp/server.log" >&2
    echo "the test server did not start" >&2
    exit 1
}
base=http://127.0.0.1:$port

opt=$home/.local/opt/canopy
apps=$home/.local/share/applications/io.necturalabs.Canopy.desktop
icon=$home/.local/share/icons/hicolor/256x256/apps/io.necturalabs.Canopy.png
svg=$home/.local/share/icons/hicolor/scalable/apps/io.necturalabs.Canopy.svg
link=$home/.local/bin/canopy

make_release 0.2.0
make_release 0.3.0
make_release 0.4.0-beta.1
publish latest 0.2.0

echo "== fresh install"
run_status
check "install exits 0" [ "$status" = 0 ]
show
check "binary installed" [ -x "$opt/bin/canopy" ]
check "VERSION is 0.2.0" [ "$(cat "$opt/VERSION")" = 0.2.0 ]
check "uninstall.sh installed" [ -x "$opt/uninstall.sh" ]
check "receipt written" [ -f "$opt/.canopy-install" ]
check "notices installed" [ -f "$opt/legal/third-party-notices.txt" ]
check "licence installed" [ -f "$opt/LICENSE" ]
check "metainfo inside the install folder" [ -f "$opt/share/metainfo/io.necturalabs.Canopy.metainfo.xml" ]
check "command symlink" [ "$(readlink "$link")" = "$opt/bin/canopy" ]
check "command runs" [ "$("$link")" = "canopy 0.2.0 (fake)" ]
check "desktop entry registered" [ -f "$apps" ]
check "desktop Exec is absolute" grep -qxF "Exec=\"$opt/bin/canopy\" %F" "$apps"
check "icons registered" [ -f "$icon" ]
check "icons registered (2)" [ -f "$svg" ]
check "no staging left" [ -z "$(find "$opt" -maxdepth 1 \( -name '.staging.*' -o -name '.previous.*' -o -name .lock \))" ]
check "PATH note shown" out_has "is not on your PATH"
check "git reported" out_has "git:"
check "checksum reported" out_has "Checksum verified"
outside=$(cd "$home" && find . \( -type f -o -type l \) | grep -v '^\./\.local/opt/canopy/' | sort | tr '\n' ' ')
check "nothing else written outside the install folder ($outside)" [ "$outside" = "./.local/bin/canopy ./.local/share/applications/io.necturalabs.Canopy.desktop ./.local/share/icons/hicolor/256x256/apps/io.necturalabs.Canopy.png ./.local/share/icons/hicolor/scalable/apps/io.necturalabs.Canopy.svg " ]

echo "== reinstall (idempotent)"
before=$(cd "$home" && find . | sort)
run_status
check "reinstall exits 0" [ "$status" = 0 ]
check "says reinstalled" out_has "Reinstalled Canopy 0.2.0"
check "same files after reinstall" [ "$before" = "$(cd "$home" && find . | sort)" ]

echo "== upgrade"
publish latest 0.3.0
run_status
check "upgrade exits 0" [ "$status" = 0 ]
show
check "says updated" out_has "Updated Canopy 0.2.0 -> 0.3.0"
check "runs 0.3.0" [ "$("$link")" = "canopy 0.3.0 (fake)" ]
check "no staging left after upgrade" [ -z "$(find "$opt" -maxdepth 1 -name '.*' ! -name .canopy-install ! -path "$opt")" ]

echo "== checksum mismatch is refused"
make_release 0.5.0
publish latest 0.5.0 0000000000000000000000000000000000000000000000000000000000000000
run_status
check "mismatch exits non-zero" [ "$status" != 0 ]
show
check "mismatch reported" out_has "checksum mismatch"
check "old version kept" [ "$("$link")" = "canopy 0.3.0 (fake)" ]
check "old version kept (2)" [ "$(cat "$opt/VERSION")" = 0.3.0 ]
check "no staging left after failure" [ -z "$(find "$opt" -maxdepth 1 -name '.*' ! -name .canopy-install ! -path "$opt")" ]
publish latest 0.3.0

echo "== tampered archive with the right name is refused"
cp "$srv/releases/download/canopy-v0.3.0/canopy-0.3.0-$target.tar.gz" "$tmp/good.tar.gz"
printf 'x' >>"$srv/releases/download/canopy-v0.3.0/canopy-0.3.0-$target.tar.gz"
run_status
check "tampered archive exits non-zero" [ "$status" != 0 ]
check "tampered archive exits non-zero (2)" out_has "checksum mismatch"
cp "$tmp/good.tar.gz" "$srv/releases/download/canopy-v0.3.0/canopy-0.3.0-$target.tar.gz"

echo "== manifest pointing elsewhere is refused"
sed 's#"url": "[^"]*"#"url": "https://example.invalid/canopy-0.3.0-'"$target"'.tar.gz"#' "$srv/canopy/latest.json" >"$tmp/m" && cp "$tmp/m" "$srv/canopy/latest.json"
run_status
check "foreign URL exits non-zero" [ "$status" != 0 ]
check "foreign URL exits non-zero (2)" out_has "outside Canopy"
publish latest 0.3.0

echo "== pinned older version"
run_status --version 0.2.0
check "--version exits 0" [ "$status" = 0 ]
check "runs 0.2.0" [ "$("$link")" = "canopy 0.2.0 (fake)" ]
run_status --version 9.9.9
check "unknown version exits non-zero" [ "$status" != 0 ]
check "unknown version exits non-zero (2)" out_has "not published"

echo "== beta channel"
publish beta 0.4.0-beta.1
run_status --beta
check "--beta exits 0" [ "$status" = 0 ]
check "runs the beta" [ "$("$link")" = "canopy 0.4.0-beta.1 (fake)" ]
run_status
check "stable again" [ "$status" = 0 ]
check "stable again (2)" [ "$("$link")" = "canopy 0.3.0 (fake)" ]

echo "== platform missing from the manifest"
sed "s/\"$key\": {\$/\"$other\": {/" "$srv/canopy/latest.json" >"$tmp/m" && cp "$tmp/m" "$srv/canopy/latest.json"
run_status
check "missing platform exits non-zero" [ "$status" != 0 ]
check "missing platform exits non-zero (2)" out_has "has no build for"
publish latest 0.3.0

echo "== --no-path"
run_status --no-path
check "--no-path exits 0" [ "$status" = 0 ]
check "symlink removed" [ ! -e "$link" ]
check "symlink removed (2)" [ ! -L "$link" ]
check "app still installed" [ -x "$opt/bin/canopy" ]
run_status
check "symlink back" [ -L "$link" ]

echo "== a foreign ~/.local/bin/canopy is left alone"
rm -f "$link"
printf 'mine\n' >"$link"
run_status
check "install exits 0" [ "$status" = 0 ]
check "install exits 0 (2)" out_has "left it alone"
check "foreign file kept" [ "$(cat "$link")" = mine ]
rm -f "$link"
run_status

echo "== test-only base URL is restricted"
set +e
env -i HOME="$home" PATH=/usr/bin:/bin CANOPY_INSTALL_BASE_URL=https://example.invalid "$SH" "$installer" >"$tmp/out" 2>&1
status=$?
set -e
check "remote override refused" [ "$status" != 0 ]
check "remote override refused (2)" out_has "only accepts http://127.0.0.1"

echo "== bad option"
run_status --frobnicate
check "unknown option exits 2" [ "$status" = 2 ]
run_status --help
check "--help exits 0" [ "$status" = 0 ]
check "--help exits 0 (2)" out_has "--uninstall"

echo "== uninstall with the installed uninstall.sh"
mkdir -p "$home/.local/share/canopy" "$home/.config/canopy"
echo settings >"$home/.local/share/canopy/app.ron"
set +e
env -i HOME="$home" PATH=/usr/bin:/bin "$SH" "$opt/uninstall.sh" >"$tmp/out" 2>&1
status=$?
set -e
check "uninstall exits 0" [ "$status" = 0 ]
show
check "install folder removed" [ ! -e "$opt" ]
check "symlink removed" [ ! -e "$link" ]
check "symlink removed (2)" [ ! -L "$link" ]
check "desktop entry removed" [ ! -e "$apps" ]
check "icons removed" [ ! -e "$icon" ]
check "icons removed (2)" [ ! -e "$svg" ]
check "user data kept" [ -f "$home/.local/share/canopy/app.ron" ]
left=$(cd "$home" && find . -type f | sort | tr '\n' ' ')
check "only user data left ($left)" [ "$left" = "./.local/share/canopy/app.ron " ]
run_status --uninstall
check "second uninstall is a no-op" [ "$status" = 0 ]
check "second uninstall is a no-op (2)" out_has "not installed"

echo "== installer piped into sh, then --uninstall --purge"
set +e
env -i HOME="$home" PATH=/usr/bin:/bin CANOPY_INSTALL_BASE_URL="$base" "$SH" -s -- <"$installer" >"$tmp/out" 2>&1
status=$?
set -e
check "piped install exits 0" [ "$status" = 0 ]
check "uninstall.sh fetched from the feed" cmp -s "$installer" "$opt/uninstall.sh"
run_status --uninstall --purge
check "--purge without a terminal or --yes refuses" [ "$status" != 0 ]
check "--purge without a terminal or --yes refuses (2)" out_has "--yes"
check "nothing removed when --purge was refused" [ -x "$opt/bin/canopy" ]
run_status --uninstall --purge --yes
check "purge exits 0" [ "$status" = 0 ]
show
check "purge exits 0 (2)" out_has "keychain"
check "purge also uninstalled" [ ! -e "$opt" ]
check "user data purged" [ ! -e "$home/.local/share/canopy" ]
check "user data purged (2)" [ ! -e "$home/.config/canopy" ]
check "home is empty of files" [ -z "$(cd "$home" && find . -type f -o -type l)" ]

echo "== a non-empty folder not made by the installer is refused"
mkdir -p "$home/elsewhere"
echo keep >"$home/elsewhere/file"
set +e
env -i HOME="$home" PATH=/usr/bin:/bin CANOPY_INSTALL_BASE_URL="$base" CANOPY_INSTALL_DIR="$home/elsewhere" \
    "$SH" "$installer" >"$tmp/out" 2>&1
status=$?
set -e
check "foreign folder refused" [ "$status" != 0 ]
check "foreign folder refused (2)" [ "$(cat "$home/elsewhere/file")" = keep ]

echo "== a failed first install leaves nothing"
rm -rf "$home"
mkdir -p "$home"
publish latest 0.5.0 0000000000000000000000000000000000000000000000000000000000000000
run_status
check "failed first install exits non-zero" [ "$status" != 0 ]
check "no install folder left" [ ! -e "$opt" ]

echo "== custom CANOPY_INSTALL_DIR with a space"
publish latest 0.3.0
set +e
env -i HOME="$home" PATH=/usr/bin:/bin CANOPY_INSTALL_BASE_URL="$base" CANOPY_INSTALL_DIR="$home/My Apps/canopy" \
    "$SH" "$installer" >"$tmp/out" 2>&1
status=$?
set -e
check "custom dir install exits 0" [ "$status" = 0 ]
check "custom dir Exec quoted" grep -qxF "Exec=\"$home/My Apps/canopy/bin/canopy\" %F" "$apps"
set +e
env -i HOME="$home" PATH=/usr/bin:/bin "$SH" "$home/My Apps/canopy/uninstall.sh" >"$tmp/out" 2>&1
status=$?
set -e
check "custom dir uninstall" [ "$status" = 0 ]
check "custom dir uninstall (2)" [ ! -e "$home/My Apps/canopy" ]
check "custom dir uninstall (3)" [ ! -e "$apps" ]

echo "== wget instead of curl"
if command -v wget >/dev/null 2>&1 && wget --version 2>/dev/null | grep -q 'GNU Wget'; then
    # A PATH with every system command except curl.
    mkdir -p "$tmp/nocurl"
    for f in /usr/local/bin/* /usr/bin/* /bin/*; do
        n=${f##*/}
        [ "$n" = curl ] || [ -e "$tmp/nocurl/$n" ] || ln -s "$f" "$tmp/nocurl/$n"
    done
    run_path=$tmp/nocurl
    run_status
    check "install with wget exits 0" [ "$status" = 0 ]
    check "installed with wget" [ "$(cat "$opt/VERSION" 2>/dev/null)" = 0.3.0 ]
    run_status --uninstall
    run_path=/usr/local/bin:/usr/bin:/bin
else
    echo "skipped: GNU wget is not installed"
fi

echo "== macOS flow, simulated on Linux with a fake uname"
# Checks the app-bundle logic (layout, bundle id, swap, symlink, uninstall), not BSD tools.
mkdir -p "$tmp/fakemac"
cat >"$tmp/fakemac/uname" <<'EOF'
#!/bin/sh
case ${1:-} in -m) echo arm64 ;; *) echo Darwin ;; esac
EOF
chmod 755 "$tmp/fakemac/uname"
run_path=$tmp/fakemac:/usr/local/bin:/usr/bin:/bin
target=aarch64-apple-darwin key=darwin-aarch64
make_mac_release() {
    v=$1
    name=canopy-$v-$target
    d=$tmp/build/$name
    app=$d/Canopy.app/Contents
    mkdir -p "$app/MacOS" "$app/Resources" "$d/legal"
    printf '#!/bin/sh\necho "canopy %s (fake mac)"\n' "$v" >"$app/MacOS/canopy"
    chmod 755 "$app/MacOS/canopy"
    sed -e "s/@VERSION@/$v/g" -e "s/@BUNDLE_VERSION@/${v%%-*}/g" "$root/packaging/macos/Info.plist" >"$app/Info.plist"
    echo "Proprietary" >"$d/LICENSE"
    rel=$srv/releases/download/canopy-v$v
    mkdir -p "$rel"
    tar -czf "$rel/$name.tar.gz" -C "$tmp/build" "$name"
}
make_mac_release 0.2.0
make_mac_release 0.3.0
rm -rf "$home"
mkdir -p "$home"
mapp=$home/Applications/Canopy.app
publish latest 0.2.0
run_status
check "mac install exits 0" [ "$status" = 0 ]
show
check "app bundle installed" [ -x "$mapp/Contents/MacOS/canopy" ]
check "mac command symlink" [ "$(readlink "$link")" = "$mapp/Contents/MacOS/canopy" ]
check "mac command runs" [ "$("$link")" = "canopy 0.2.0 (fake mac)" ]
publish latest 0.3.0
run_status
check "mac upgrade exits 0" [ "$status" = 0 ]
check "mac says updated" out_has "Updated Canopy 0.2.0 -> 0.3.0"
check "mac runs 0.3.0" [ "$("$link")" = "canopy 0.3.0 (fake mac)" ]
check "no temporary folders left in ~/Applications" [ "$(ls -A "$home/Applications")" = Canopy.app ]
publish latest 0.5.0 0000000000000000000000000000000000000000000000000000000000000000
make_mac_release 0.5.0
run_status
check "mac mismatch refused" [ "$status" != 0 ]
check "mac old version kept" [ "$("$link")" = "canopy 0.3.0 (fake mac)" ]
mkdir -p "$home/Other.app/Contents"
sed 's/io.necturalabs.Canopy/com.example.Other/' "$mapp/Contents/Info.plist" >"$home/Other.app/Contents/Info.plist"
set +e
env -i HOME="$home" PATH="$run_path" CANOPY_INSTALL_BASE_URL="$base" CANOPY_INSTALL_DIR="$home/Other.app" \
    "$SH" "$installer" --uninstall >"$tmp/out" 2>&1
status=$?
set -e
check "another app is not removed" [ "$status" != 0 ]
check "another app is not removed (2)" [ -d "$home/Other.app" ]
run_status --uninstall
check "mac uninstall exits 0" [ "$status" = 0 ]
check "app bundle removed" [ ! -e "$mapp" ]
check "mac symlink removed" [ ! -e "$link" ]
check "mac symlink removed (2)" [ ! -L "$link" ]

echo
if [ "$failures" -ne 0 ]; then
    echo "$failures check(s) failed (shell: $SH). Last installer output:"
    cat "$tmp/out"
    exit 1
fi
echo "All installer checks passed (shell: $SH)."
