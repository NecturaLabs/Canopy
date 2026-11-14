#!/bin/sh
# Usage: packaging/test-install.sh
#
# End-to-end test of packaging/install.sh against a fake update feed: builds fake release archives
# (a dummy `canopy` script, or a dummy Canopy.app), manifests and SHA256SUMS, serves them with
# `python3 -m http.server` on 127.0.0.1, and runs the installer with a temporary HOME through its
# test-only CANOPY_INSTALL_BASE_URL. Never touches the real HOME. Needs python3, curl and tar.
#
# On Linux it runs the Linux flow, then the macOS flow under a fake `uname`; on macOS it runs the
# macOS flow natively. CANOPY_TEST_SHELL picks the shell that runs the installer (default: dash
# when installed, else sh), e.g. CANOPY_TEST_SHELL=bash or CANOPY_TEST_SHELL="busybox sh".
set -eu

root=$(cd "$(dirname "$0")/.." && pwd -P)
installer=$root/packaging/install.sh
os=$(uname -s)
case $os in
    Linux | Darwin) ;;
    *)
        echo "test-install.sh runs on Linux and macOS only" >&2
        exit 1
        ;;
esac

# The name holds a space on purpose: the installer and this script must cope with a path that has
# one, as a mounted disk's "/run/media/<user>/Programming SSD" does.
tmp=$(mktemp -d "${TMPDIR:-/tmp}/canopy install test.XXXXXX")
# The real path: the installer records real paths (pwd -P), and macOS's /tmp and TMPDIR are
# symlinks into /private.
tmp=$(cd "$tmp" && pwd -P)
server_pid=
cleanup() {
    if [ -n "$server_pid" ]; then kill "$server_pid" 2>/dev/null || true; fi
    rm -rf -- "$tmp"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# The shell that runs the installer, wrapped in a script so that a command with arguments, such
# as "busybox sh", works everywhere a shell path does.
sh_cmd='sh'
if command -v dash >/dev/null 2>&1; then sh_cmd=dash; fi
sh_cmd=${CANOPY_TEST_SHELL:-$sh_cmd}
sh_first=${sh_cmd%% *}
sh_rest=${sh_cmd#"$sh_first"}
sh_abs=$(command -v "$sh_first") || {
    echo "no such shell: $sh_first" >&2
    exit 1
}
SH=$tmp/test-shell
printf '#!/bin/sh\nexec "%s"%s "$@"\n' "$sh_abs" "$sh_rest" >"$SH"
chmod 755 "$SH"

real_mv=$(command -v mv)
srv=$tmp/srv
mkdir -p "$srv/canopy" "$srv/releases/download"
cp "$installer" "$srv/canopy/install.sh"

checks=0
failures=0
pass() { printf 'ok   %s\n' "$*"; }
fail() {
    printf 'FAIL %s\n' "$*"
    failures=$((failures + 1))
}
check() {
    desc=$1
    shift
    checks=$((checks + 1))
    if "$@"; then pass "$desc"; else fail "$desc"; fi
}

sha() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum -- "$1" | awk '{print $1}'
    else
        shasum -a 256 -- "$1" | awk '{print $1}'
    fi
}

# Writes the SHA256SUMS of release folder $1.
write_sums() {
    for f in "$1"/*.tar.gz; do
        printf '%s  %s\n' "$(sha "$f")" "${f##*/}"
    done >"$1/SHA256SUMS"
}

# Builds Linux release $1 into the fake feed.
make_release() {
    v=$1
    name=canopy-build-$v-$target
    d=$tmp/build/$name
    rm -rf "$d"
    mkdir -p "$d/legal"
    printf '#!/bin/sh\necho "canopy %s (fake)"\n' "$v" >"$d/canopy"
    chmod 755 "$d/canopy"
    echo "Canopy" >"$d/README.md"
    echo "Proprietary" >"$d/LICENSE"
    echo "notices" >"$d/legal/third-party-notices.txt"
    cp "$root/packaging/linux/io.necturalabs.Canopy.desktop" "$root/packaging/linux/io.necturalabs.Canopy.metainfo.xml" "$d/"
    # A second group with its own Exec=, which the installer must leave alone.
    printf '\n[Desktop Action new-window]\nName=New Window\nExec=canopy --new-window\n' >>"$d/io.necturalabs.Canopy.desktop"
    cp "$root/assets/icon-256.png" "$d/io.necturalabs.Canopy.png"
    rel=$srv/releases/download/canopy-build-$v
    mkdir -p "$rel"
    tar -czf "$rel/$name.tar.gz" -C "$tmp/build" "$name"
    write_sums "$rel"
}

# Builds macOS release $1 into the fake feed.
make_mac_release() {
    v=$1
    name=canopy-build-$v-$target
    d=$tmp/build/$name
    rm -rf "$d"
    app=$d/Canopy.app/Contents
    mkdir -p "$app/MacOS" "$app/Resources" "$d/legal"
    printf '#!/bin/sh\necho "canopy %s (fake mac)"\n' "$v" >"$app/MacOS/canopy"
    chmod 755 "$app/MacOS/canopy"
    sed -e "s/@BUILD@/$v/g" "$root/packaging/macos/Info.plist" >"$app/Info.plist"
    echo "Proprietary" >"$d/LICENSE"
    rel=$srv/releases/download/canopy-build-$v
    mkdir -p "$rel"
    tar -czf "$rel/$name.tar.gz" -C "$tmp/build" "$name"
    write_sums "$rel"
}

# Writes channel manifest $1 for version $2; $3 overrides the checksum.
publish() {
    ch=$1 v=$2
    name=canopy-build-$v-$target
    digest=${3:-$(sha "$srv/releases/download/canopy-build-$v/$name.tar.gz")}
    cat >"$srv/canopy/$ch.json" <<EOF
{
  "version": "$v",
  "notes": "Notes with \"quotes\", braces { } and a fake \"platforms\": {\"$key\": {\"url\": \"https://evil.invalid/x\"}}\\n- line",
  "pub_date": "2026-10-01T00:00:00Z",
  "release_page": "$base/releases/tag/canopy-build-$v",
  "platforms": {
    "$key": {
      "url": "$base/releases/download/canopy-build-$v/$name.tar.gz",
      "sha256": "$digest"
    }
  }
}
EOF
}

# Runs a command with a clean environment and the temporary HOME; output in $tmp/out, exit
# status in $status. run_raw takes extra NAME=value pairs before the command.
run_path=/usr/local/bin:/usr/bin:/bin
run_raw() {
    set +e
    env -i HOME="$home" PATH="$run_path" "$@" >"$tmp/out" 2>&1
    status=$?
    set -e
}
# The installer against the fake feed.
run_status() { run_raw CANOPY_INSTALL_BASE_URL="$base" "$SH" "$installer" "$@"; }
out_has() { grep -qF -- "$1" "$tmp/out"; }
out_lacks() { ! grep -qiF -- "$1" "$tmp/out"; }
# Shows the installer output of the step above, indented, so the log reads like a session.
show() { sed 's/^/  | /' "$tmp/out"; }
# Whether line $2 of desktop entry $1 sits in its [Desktop Entry] group, before the action group.
marker_in_main_group() {
    awk -v m="$2" '
        $0 == "[Desktop Entry]" { g = NR } $0 == m { mk = NR } /^\[Desktop Action/ { ac = NR }
        END { exit !(g && mk > g && ac > mk) }' "$1"
}
is_empty_dir() { [ -d "$1" ] && [ -z "$(ls -A "$1")" ]; }
no_leftovers() { [ -z "$(find "$1" -maxdepth 1 \( -name '.staging.*' -o -name '.previous.*' -o -name .lock -o -name '.canopy-install.new' \))" ]; }

# A PATH holding every command of the usual system folders except those named in $2..., in
# folder $1.
path_without() {
    dir=$1
    shift
    mkdir -p "$dir"
    for f in /usr/local/bin/* /usr/bin/* /bin/*; do
        n=${f##*/}
        skip=0
        for x in "$@"; do
            if [ "$n" = "$x" ]; then skip=1; fi
        done
        [ "$skip" = 1 ] || [ -e "$dir/$n" ] || [ -L "$dir/$n" ] || ln -s "$f" "$dir/$n"
    done
}

# A stub `mv` in folder $1 for the installer's `mv -- SRC DST`: the first time SRC (or DST, when
# $2 is "dst") matches case pattern $3, it does $4: "fail" (exit 1), or "interrupt" (move, then
# send SIGINT to the installer, as Ctrl+C would). In $3 only `*` is a wildcard; every other
# character that is not a plain path character is quoted, so a space in the temporary folder's
# name (a TMPDIR such as "/mnt/My Disk/tmp") cannot turn the stub into a syntax error.
stub_mv() {
    mkdir -p "$1"
    pat=$(printf '%s\n' "$3" | sed 's|[^A-Za-z0-9_.*/-]|\\&|g')
    cat >"$1/mv" <<EOF
#!/bin/sh
real="$real_mv"
flag="$1/fired"
arg=
if [ "\$1" = -- ]; then
    if [ "$2" = dst ]; then arg=\$3; else arg=\$2; fi
fi
if [ ! -e "\$flag" ]; then
    case \$arg in
        $pat)
            : >"\$flag"
            if [ "$4" = fail ]; then
                echo "mv: simulated failure" >&2
                exit 1
            fi
            "\$real" "\$@"
            kill -INT "\$PPID"
            exit 0
            ;;
    esac
fi
exec "\$real" "\$@"
EOF
    chmod 755 "$1/mv"
}

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

# ---------------------------------------------------------------------------------------------
linux_flow() {
    case $(uname -m) in
        x86_64 | amd64) target=x86_64-unknown-linux-gnu key=linux-x86_64 other=linux-aarch64 ;;
        aarch64 | arm64) target=aarch64-unknown-linux-gnu key=linux-aarch64 other=linux-x86_64 ;;
        *)
            echo "unsupported test machine: $(uname -m)" >&2
            exit 1
            ;;
    esac
    home=$tmp/home
    mkdir -p "$home"
    opt=$home/.local/opt/canopy
    apps=$home/.local/share/applications/io.necturalabs.Canopy.desktop
    icon=$home/.local/share/icons/hicolor/256x256/apps/io.necturalabs.Canopy.png
    svg=$home/.local/share/icons/hicolor/scalable/apps/io.necturalabs.Canopy.svg
    link=$home/.local/bin/canopy

    make_release 102
    make_release 103
    make_release 105
    publish latest 102

    echo "== fresh install"
    run_status
    check "install exits 0" [ "$status" = 0 ]
    show
    check "binary installed" [ -x "$opt/bin/canopy" ]
    check "VERSION is 102" [ "$(cat "$opt/VERSION")" = 102 ]
    check "uninstall.sh installed" [ -x "$opt/uninstall.sh" ]
    check "receipt written" [ -f "$opt/.canopy-install" ]
    check "receipt records the folder" grep -qxF "root=$opt" "$opt/.canopy-install"
    check "notices installed" [ -f "$opt/legal/third-party-notices.txt" ]
    check "licence installed" [ -f "$opt/LICENSE" ]
    check "metainfo inside the install folder" [ -f "$opt/share/metainfo/io.necturalabs.Canopy.metainfo.xml" ]
    check "command symlink" [ "$(readlink "$link")" = "$opt/bin/canopy" ]
    check "command runs" [ "$("$link")" = "canopy 102 (fake)" ]
    check "desktop entry registered" [ -f "$apps" ]
    check "desktop Exec is absolute" grep -qxF "Exec=\"$opt/bin/canopy\" %F" "$apps"
    check "desktop action keeps its own Exec" grep -qxF "Exec=canopy --new-window" "$apps"
    check "install marker inside [Desktop Entry]" marker_in_main_group "$apps" "X-Canopy-Install-Dir=$opt"
    check "icons registered" [ -f "$icon" ]
    check "no scalable icon any more" [ ! -e "$svg" ]
    check "no staging left" no_leftovers "$opt"
    check "PATH note shown" out_has "is not on your PATH"
    check "git reported" out_has "git:"
    check "checksum reported" out_has "Checksum verified"
    outside=$(cd "$home" && find . \( -type f -o -type l \) | grep -v '^\./\.local/opt/canopy/' | sort | tr '\n' ' ')
    check "nothing else written outside the install folder ($outside)" [ "$outside" = "./.local/bin/canopy ./.local/share/applications/io.necturalabs.Canopy.desktop ./.local/share/icons/hicolor/256x256/apps/io.necturalabs.Canopy.png " ]

    echo "== the scalable icon earlier builds registered"
    # The test's installer carries the digest of this stand-in for a shipped icon.
    shipped_text="an icon Canopy shipped"
    shipped_sha=$(printf '%s\n' "$shipped_text" | sha256sum | awk '{print $1}')
    sed "s/^LEGACY_SVG_SHA256=.*/LEGACY_SVG_SHA256=\"$shipped_sha\"/" "$installer" >"$tmp/install-shipped.sh"
    real_installer=$installer
    installer=$tmp/install-shipped.sh
    printf 'icon=%s|%s\n' "$svg" "$opt/share/icons/hicolor/scalable/apps/io.necturalabs.Canopy.svg" >>"$opt/.canopy-install"
    mkdir -p "$(dirname "$svg")"
    printf '%s\n' "$shipped_text" >"$svg"
    # One registered while XDG_DATA_HOME pointed elsewhere.
    svg_a=$home/data-a/icons/hicolor/scalable/apps/io.necturalabs.Canopy.svg
    mkdir -p "$(dirname "$svg_a")"
    printf '%s\n' "$shipped_text" >"$svg_a"
    printf 'icon=%s|%s\n' "$svg_a" "$opt/share/icons/hicolor/scalable/apps/io.necturalabs.Canopy.svg" >>"$opt/.canopy-install"
    run_status
    check "install exits 0" [ "$status" = 0 ]
    check "old scalable icon removed" [ ! -e "$svg" ]
    check "old scalable icon removed where it was registered" [ ! -e "$svg_a" ]
    check "and dropped from the receipt" [ -z "$(grep -F "scalable" "$opt/.canopy-install")" ]

    echo "== an edited scalable icon still named in the old receipt"
    printf '%s\n' "$shipped_text" >"$svg"
    echo "edited" >>"$svg"
    printf 'icon=%s|%s\n' "$svg" "$opt/share/icons/hicolor/scalable/apps/io.necturalabs.Canopy.svg" >>"$opt/.canopy-install"
    run_status
    check "install exits 0 with an edited registered SVG" [ "$status" = 0 ]
    check "edited registered SVG is preserved" [ "$(cat "$svg")" = "$shipped_text
edited" ]
    check "edited SVG is not claimed in the receipt" [ -z "$(grep -F "scalable" "$opt/.canopy-install")" ]
    rm "$svg"

    echo "== a custom SVG absent from the receipt"
    # A receipt for the neighbouring PNG does not establish who installed this SVG.
    echo "user's custom svg" >"$svg"
    run_status
    check "install exits 0 with an unregistered SVG" [ "$status" = 0 ]
    check "custom SVG is preserved despite the registered PNG" [ "$(cat "$svg" 2>/dev/null)" = "user's custom svg" ]
    check "custom SVG is not claimed in the receipt" [ -z "$(grep -F "scalable" "$opt/.canopy-install")" ]
    rm "$svg"
    echo "another user's svg" >"$tmp/untouched.svg"
    ln -s "$tmp/untouched.svg" "$svg"
    run_status
    check "install exits 0 with symlinked SVG" [ "$status" = 0 ]
    check "unregistered symlink is preserved" [ -L "$svg" ]
    check "symlink target is preserved" [ "$(cat "$tmp/untouched.svg")" = "another user's svg" ]
    rm "$svg" "$tmp/untouched.svg"

    echo "== an orphaned scalable icon whose content Canopy shipped"
    # The receipt no longer lists it (a build that rewrote the receipt dropped the line), but
    # the content proves where it came from.
    printf '%s\n' "$shipped_text" >"$svg"
    run_status
    check "install exits 0 with an orphaned shipped icon" [ "$status" = 0 ]
    check "orphaned shipped icon removed" [ ! -e "$svg" ]
    printf '%s\n' "$shipped_text" >"$svg"
    echo "edited" >>"$svg"
    run_status
    check "an edited copy of the shipped icon is preserved" [ -f "$svg" ]
    rm "$svg"
    printf '%s\n' "$shipped_text" >"$tmp/shipped-target"
    ln -s "$tmp/shipped-target" "$svg"
    run_status
    check "a symlink to the shipped icon is preserved" [ -L "$svg" ]
    rm "$svg" "$tmp/shipped-target"
    installer=$real_installer
    rmdir "$(dirname "$svg")" "$(dirname "$(dirname "$svg")")"
    rm -rf "$home/data-a"

    echo "== reinstall (idempotent)"
    before=$(cd "$home" && find . | sort)
    run_status
    check "reinstall exits 0" [ "$status" = 0 ]
    check "says reinstalled" out_has "Reinstalled Canopy build 102"
    check "same files after reinstall" [ "$before" = "$(cd "$home" && find . | sort)" ]

    echo "== upgrade"
    echo "my notes" >"$opt/notes.txt"
    publish latest 103
    run_status
    check "upgrade exits 0" [ "$status" = 0 ]
    show
    check "says updated" out_has "Updated Canopy build 102 -> build 103"
    check "runs 103" [ "$("$link")" = "canopy 103 (fake)" ]
    check "no staging left after upgrade" no_leftovers "$opt"
    check "the user's file survives an upgrade" [ "$(cat "$opt/notes.txt" 2>/dev/null)" = "my notes" ]
    rm -f "$opt/notes.txt"

    echo "== checksum mismatch is refused"
    make_release 106
    publish latest 106 0000000000000000000000000000000000000000000000000000000000000000
    run_status
    check "mismatch exits non-zero" [ "$status" != 0 ]
    show
    check "mismatch reported" out_has "checksum mismatch"
    check "old version kept" [ "$("$link")" = "canopy 103 (fake)" ]
    check "old version kept (2)" [ "$(cat "$opt/VERSION")" = 103 ]
    check "no staging left after failure" no_leftovers "$opt"
    publish latest 103

    echo "== tampered archive with the right name is refused"
    cp "$srv/releases/download/canopy-build-103/canopy-build-103-$target.tar.gz" "$tmp/good.tar.gz"
    printf 'x' >>"$srv/releases/download/canopy-build-103/canopy-build-103-$target.tar.gz"
    run_status
    check "tampered archive exits non-zero" [ "$status" != 0 ]
    check "tampered archive exits non-zero (2)" out_has "checksum mismatch"
    cp "$tmp/good.tar.gz" "$srv/releases/download/canopy-build-103/canopy-build-103-$target.tar.gz"

    echo "== manifest pointing elsewhere is refused"
    sed 's#"url": "[^"]*"#"url": "https://example.invalid/canopy-build-103-'"$target"'.tar.gz"#' "$srv/canopy/latest.json" >"$tmp/m" && cp "$tmp/m" "$srv/canopy/latest.json"
    run_status
    check "foreign URL exits non-zero" [ "$status" != 0 ]
    check "foreign URL exits non-zero (2)" out_has "outside Canopy"
    publish latest 103

    echo "== a move that fails while the old version is moved aside"
    stub_mv "$tmp/mv-aside" src "$opt/legal" fail
    run_path=$tmp/mv-aside:/usr/local/bin:/usr/bin:/bin
    run_status
    run_path=/usr/local/bin:/usr/bin:/bin
    check "failed swap exits non-zero" [ "$status" != 0 ]
    check "failed swap reported" out_has "the previous one was kept"
    check "old version intact" [ "$("$link")" = "canopy 103 (fake)" ]
    check "old version intact (VERSION)" [ "$(cat "$opt/VERSION")" = 103 ]
    check "old version intact (entry not yet moved)" [ -f "$opt/legal/third-party-notices.txt" ]
    check "old version intact (moved entries back)" [ -x "$opt/uninstall.sh" ]
    check "old version intact (share)" [ -f "$opt/share/applications/io.necturalabs.Canopy.desktop" ]
    check "no leftovers after the failed swap" no_leftovers "$opt"

    echo "== a move that fails while the new version is moved in"
    make_release 104
    publish latest 104
    stub_mv "$tmp/mv-in" src "$opt/.staging.*/new/legal" fail
    run_path=$tmp/mv-in:/usr/local/bin:/usr/bin:/bin
    run_status
    run_path=/usr/local/bin:/usr/bin:/bin
    check "failed swap exits non-zero" [ "$status" != 0 ]
    check "old version intact" [ "$("$link")" = "canopy 103 (fake)" ]
    check "old version intact (VERSION)" [ "$(cat "$opt/VERSION")" = 103 ]
    check "old version intact (legal)" [ -f "$opt/legal/third-party-notices.txt" ]
    check "old version intact (uninstall.sh)" [ -x "$opt/uninstall.sh" ]
    check "no leftovers after the failed swap" no_leftovers "$opt"

    echo "== Ctrl+C while the new version is moved in"
    stub_mv "$tmp/mv-int" src "$opt/.staging.*/new/share" interrupt
    run_path=$tmp/mv-int:/usr/local/bin:/usr/bin:/bin
    run_status
    run_path=/usr/local/bin:/usr/bin:/bin
    check "interrupted install exits 130" [ "$status" = 130 ]
    check "old version put back" [ "$("$link")" = "canopy 103 (fake)" ]
    check "old version put back (VERSION)" [ "$(cat "$opt/VERSION")" = 103 ]
    check "old version put back (share)" [ -f "$opt/share/applications/io.necturalabs.Canopy.desktop" ]
    check "old version put back (uninstall.sh)" [ -x "$opt/uninstall.sh" ]
    check "no leftovers after the interrupt" no_leftovers "$opt"
    publish latest 103

    echo "== pinned older version"
    run_status --version 102
    check "--version exits 0" [ "$status" = 0 ]
    check "runs 102" [ "$("$link")" = "canopy 102 (fake)" ]
    run_status --version 999
    check "unknown version exits non-zero" [ "$status" != 0 ]
    check "unknown version exits non-zero (2)" out_has "not published"
    run_status --version "$(printf '103\nx')"
    check "a version with two lines is refused" [ "$status" != 0 ]
    check "a version with two lines is refused (2)" out_has "is not a build number"
    run_status
    check "back to the newest" [ "$("$link")" = "canopy 103 (fake)" ]

    echo "== beta channel, and no silent downgrade"
    publish beta 105
    run_status --beta
    check "--beta exits 0" [ "$status" = 0 ]
    check "runs the beta" [ "$("$link")" = "canopy 105 (fake)" ]
    run_status
    check "the stable channel does not downgrade the beta" [ "$status" != 0 ]
    show
    check "downgrade refusal suggests --beta" out_has "--beta"
    check "downgrade refusal suggests --version" out_has "--version 103"
    check "beta still installed" [ "$("$link")" = "canopy 105 (fake)" ]
    run_status --version 103
    check "--version goes back to stable" [ "$status" = 0 ]
    check "--version goes back to stable (2)" [ "$("$link")" = "canopy 103 (fake)" ]

    echo "== builds are ordered as integers"
    make_release 99
    make_release 100
    publish latest 99
    run_status --version 99
    check "an older build can be pinned" [ "$status" = 0 ]
    check "an older build can be pinned (2)" [ "$("$link")" = "canopy 99 (fake)" ]
    publish latest 100
    run_status
    check "build 100 replaces build 99" [ "$status" = 0 ]
    check "build 100 replaces build 99 (2)" [ "$("$link")" = "canopy 100 (fake)" ]
    publish latest 99
    run_status
    check "build 99 does not replace build 100" [ "$status" != 0 ]
    check "build 99 does not replace build 100 (2)" [ "$("$link")" = "canopy 100 (fake)" ]
    run_status --version 0.2.0
    check "a semantic version cannot be pinned" [ "$status" != 0 ]
    check "a semantic version cannot be pinned (2)" out_has "is not a build number"
    run_status --version 0100
    check "a build number with a leading zero is refused" [ "$status" != 0 ]

    echo "== an installation from before builds were numbered"
    publish latest 103
    printf '0.2.0-beta.1\n' >"$opt/VERSION"
    run_status
    check "an old semantic version is older than any build" [ "$status" = 0 ]
    check "an old semantic version is older than any build (2)" out_has "Updated Canopy 0.2.0-beta.1 -> build 103"
    check "an old semantic version is older than any build (3)" [ "$(cat "$opt/VERSION")" = 103 ]
    sed 's/"version": "103"/"version": "0.4.0-beta.1"/' "$srv/canopy/latest.json" >"$srv/canopy/beta.json"
    run_status --beta
    check "a beta manifest with a semantic version is ignored" [ "$status" = 0 ]
    check "a beta manifest with a semantic version is ignored (2)" out_has "Reinstalled Canopy build 103"
    publish beta 105

    echo "== platform missing from the manifest"
    sed "s/\"$key\": {\$/\"$other\": {/" "$srv/canopy/latest.json" >"$tmp/m" && cp "$tmp/m" "$srv/canopy/latest.json"
    run_status
    check "missing platform exits non-zero" [ "$status" != 0 ]
    check "missing platform exits non-zero (2)" out_has "has no archive for"
    publish latest 103

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
    for bad in https://example.invalid http://127.0.0.1:1@example.invalid http://localhost:1@x \
        http://127.0.0.1: http://127.0.0.1:1/path http://localhost:1// http://127.0.0.1:123456 \
        "$(printf 'http://127.0.0.1:1\nx')"; do
        run_raw CANOPY_INSTALL_BASE_URL="$bad" "$SH" "$installer"
        check "base URL refused: $bad" [ "$status" = 1 ]
        check "base URL refused before anything else: $bad" out_has "only accepts http://127.0.0.1"
    done
    run_raw CANOPY_INSTALL_BASE_URL="$base/" "$SH" "$installer"
    check "a trailing slash is accepted" [ "$status" = 0 ]

    echo "== no curl and no wget"
    path_without "$tmp/nodl" curl wget
    run_path=$tmp/nodl
    run_status
    run_path=/usr/local/bin:/usr/bin:/bin
    check "no downloader exits non-zero" [ "$status" != 0 ]
    show
    check "no downloader is explained" out_has "needs curl or GNU wget"
    check "nothing changed without a downloader" [ "$(cat "$opt/VERSION")" = 103 ]

    echo "== bad option"
    run_status --frobnicate
    check "unknown option exits 2" [ "$status" = 2 ]
    run_status --help
    check "--help exits 0" [ "$status" = 0 ]
    check "--help exits 0 (2)" out_has "--uninstall"

    echo "== a file of the user's in the install folder survives uninstall"
    echo "my notes" >"$opt/notes.txt"
    run_raw "$SH" "$opt/uninstall.sh"
    check "uninstall exits 0" [ "$status" = 0 ]
    show
    check "the user's file is kept" [ "$(cat "$opt/notes.txt" 2>/dev/null)" = "my notes" ]
    check "it is reported" out_has "notes.txt"
    check "Canopy's own files are gone" [ "$(ls -A "$opt")" = notes.txt ]
    check "outside entries removed" [ ! -e "$apps" ]
    rm -rf "$opt"
    run_status

    echo "== a shell without test -O refuses to uninstall instead of skipping the owner check"
    # -Z is no operator in any shell, so the mutated copy behaves like a shell that lacks -O.
    sed 's/\[ -O /[ -Z /g' "$installer" >"$tmp/install-no-owner-test.sh"
    check "the mutated copy has no -O test left" [ -z "$(grep -F '[ -O ' "$tmp/install-no-owner-test.sh" || true)" ]
    run_raw "$SH" "$tmp/install-no-owner-test.sh" --uninstall
    check "uninstall is refused" [ "$status" != 0 ]
    check "it says why" out_has "belongs to another user"
    check "the install folder is untouched" [ -x "$opt/bin/canopy" ]
    rm -f "$tmp/install-no-owner-test.sh"

    echo "== a crafted receipt cannot make uninstall delete other files"
    echo "keep me" >"$home/victim.png"
    printf '[Desktop Entry]\nX-Canopy-Install-Dir=%s\n' "$opt" >"$home/victim.desktop"
    ln -s "$opt/bin/canopy" "$home/victim-link"
    {
        printf 'icon=%s|%s\n' "$home/victim.png" "$home/victim.png"
        printf 'desktop=%s\n' "$home/victim.desktop"
        printf 'link=%s\n' "$home/victim-link"
    } >>"$opt/.canopy-install"
    run_raw "$SH" "$opt/uninstall.sh"
    check "uninstall exits 0" [ "$status" = 0 ]
    check "a file named by a crafted icon line survives" [ "$(cat "$home/victim.png" 2>/dev/null)" = "keep me" ]
    check "a file named by a crafted desktop line survives" [ -f "$home/victim.desktop" ]
    check "a link named by a crafted link line survives" [ -L "$home/victim-link" ]
    check "Canopy's own entries are still removed" [ ! -e "$opt" ]
    check "the real icon is removed" [ ! -e "$icon" ]
    rm -f "$home/victim.png" "$home/victim.desktop" "$home/victim-link"
    run_status

    echo "== a command link that cannot be made does not lose the receipt"
    home2=$tmp/home-nolink
    mkdir -p "$home2/.local"
    echo "not a folder" >"$home2/.local/bin"
    run_raw HOME="$home2" CANOPY_INSTALL_BASE_URL="$base" "$SH" "$installer"
    check "install still exits 0" [ "$status" = 0 ]
    show
    check "the missing command is reported" out_has "could not create"
    check "the receipt still lists the desktop entry" grep -q '^desktop=' "$home2/.local/opt/canopy/.canopy-install"
    rm -rf "$home2"

    echo "== uninstall with the installed uninstall.sh"
    # As an earlier build left it: a scalable SVG inside the install and its registered copy.
    mkdir -p "$opt/share/icons/hicolor/scalable/apps" "$(dirname "$svg")"
    echo "old svg" >"$opt/share/icons/hicolor/scalable/apps/io.necturalabs.Canopy.svg"
    echo "old svg" >"$svg"
    printf 'icon=%s|%s\n' "$svg" "$opt/share/icons/hicolor/scalable/apps/io.necturalabs.Canopy.svg" >>"$opt/.canopy-install"
    mkdir -p "$home/.local/share/canopy" "$home/.config/canopy"
    echo '{"canopy-settings-v1":"(theme:Dark)"}' >"$home/.local/share/canopy/app.ron"
    run_raw "$SH" "$opt/uninstall.sh"
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
    run_raw CANOPY_INSTALL_BASE_URL="$base" "$SH" -s -- <"$installer"
    check "piped install exits 0" [ "$status" = 0 ]
    check "uninstall.sh fetched from the feed" cmp -s "$installer" "$opt/uninstall.sh"
    touch "$home/.config/canopy/.canopy-data"
    echo "settings" >"$home/.config/canopy/settings.toml"
    mkdir -p "$home/.cache/canopy" "$home/.local/state"
    echo "another program's" >"$home/.cache/canopy/other.txt"
    mkdir -p "$tmp/marked"
    touch "$tmp/marked/.canopy-data"
    ln -s "$tmp/marked" "$home/.local/state/canopy"
    run_status --uninstall --purge
    check "--purge without a terminal or --yes refuses" [ "$status" != 0 ]
    check "--purge without a terminal or --yes refuses (2)" out_has "--yes"
    check "nothing removed when --purge was refused" [ -x "$opt/bin/canopy" ]
    run_status --uninstall --purge --yes
    check "purge exits 0" [ "$status" = 0 ]
    show
    check "purge says nothing about saved tokens" out_lacks "token"
    check "purge also uninstalled" [ ! -e "$opt" ]
    check "eframe folder with Canopy's settings purged" [ ! -e "$home/.local/share/canopy" ]
    check "marked folder purged" [ ! -e "$home/.config/canopy" ]
    check "unmarked generic folder kept" [ -f "$home/.cache/canopy/other.txt" ]
    check "unmarked generic folder listed" out_has "$home/.cache/canopy"
    check "symlinked folder kept" [ -L "$home/.local/state/canopy" ]
    check "symlink target untouched" [ -f "$tmp/marked/.canopy-data" ]
    rm -rf "$home/.cache/canopy" "$home/.local/state/canopy"
    mkdir -p "$home/.local/share/canopy"
    echo "another eframe app" >"$home/.local/share/canopy/app.ron"
    run_status --uninstall --purge --yes
    check "eframe folder without Canopy's files kept" [ -f "$home/.local/share/canopy/app.ron" ]
    rm -rf "$home/.local/share/canopy"
    check "home is empty of files" [ -z "$(cd "$home" && find . -type f -o -type l)" ]

    echo "== a non-empty folder not made by the installer is refused"
    mkdir -p "$home/elsewhere"
    echo keep >"$home/elsewhere/file"
    run_raw CANOPY_INSTALL_BASE_URL="$base" CANOPY_INSTALL_DIR="$home/elsewhere" "$SH" "$installer"
    check "foreign folder refused" [ "$status" != 0 ]
    check "foreign folder refused (2)" [ "$(cat "$home/elsewhere/file")" = keep ]

    echo "== a failed install into an existing empty folder does not claim it"
    mkdir -p "$home/empty"
    publish latest 106 0000000000000000000000000000000000000000000000000000000000000000
    run_raw CANOPY_INSTALL_BASE_URL="$base" CANOPY_INSTALL_DIR="$home/empty" "$SH" "$installer"
    check "failed install exits non-zero" [ "$status" != 0 ]
    check "the folder is kept, empty" is_empty_dir "$home/empty"

    echo "== a failed first install leaves nothing"
    rm -rf "$home"
    mkdir -p "$home"
    run_status
    check "failed first install exits non-zero" [ "$status" != 0 ]
    check "no install folder left" [ ! -e "$opt" ]
    check "no ~/.local left" [ ! -e "$home/.local" ]

    echo "== a desktop entry and icons the installer does not own"
    publish latest 103
    mkdir -p "$(dirname "$apps")" "$(dirname "$svg")" "$(dirname "$icon")"
    printf '[Desktop Entry]\nName=Mine\nExec=mine\n' >"$apps"
    echo "other icon" >"$tmp/other.svg"
    ln -s "$tmp/other.svg" "$svg"
    echo "my png" >"$icon"
    run_status
    check "install exits 0" [ "$status" = 0 ]
    show
    check "foreign desktop entry kept" grep -qxF "Exec=mine" "$apps"
    check "foreign desktop entry reported" out_has "not this installer's desktop entry"
    check "symlinked icon not written through" [ "$(cat "$tmp/other.svg")" = "other icon" ]
    check "symlinked icon kept" [ -L "$svg" ]
    check "foreign icon kept" [ "$(cat "$icon")" = "my png" ]
    check "none of them in the receipt" [ -z "$(grep -E '^(desktop|icon)=' "$opt/.canopy-install")" ]
    rm -f "$apps"
    ln -s "$tmp/other.desktop" "$apps"
    echo "other entry" >"$tmp/other.desktop"
    run_status
    check "symlinked desktop entry not written through" [ "$(cat "$tmp/other.desktop")" = "other entry" ]
    run_raw "$SH" "$opt/uninstall.sh"
    check "uninstall leaves the foreign desktop entry" [ -L "$apps" ]
    check "uninstall leaves the foreign icons" [ -L "$svg" ]
    check "uninstall leaves the foreign icons (2)" [ "$(cat "$icon")" = "my png" ]
    rm -rf "$home"
    mkdir -p "$home"

    echo "== HOME is a symlink"
    realhome=$tmp/realhome
    mkdir -p "$realhome"
    rm -rf "$home"
    ln -s "$realhome" "$home"
    run_status
    check "install exits 0" [ "$status" = 0 ]
    ropt=$realhome/.local/opt/canopy
    check "receipt records the real folder" grep -qxF "root=$ropt" "$ropt/.canopy-install"
    check "desktop marker names the real folder" grep -qxF "X-Canopy-Install-Dir=$ropt" "$apps"
    check "command links the real folder" [ "$(readlink "$link")" = "$ropt/bin/canopy" ]
    run_raw "$SH" "$opt/uninstall.sh"
    check "uninstall through the symlinked HOME exits 0" [ "$status" = 0 ]
    check "desktop entry removed" [ ! -e "$apps" ]
    check "command removed" [ ! -L "$link" ]
    check "icons removed" [ ! -e "$icon" ]
    check "folder removed" [ ! -e "$ropt" ]
    rm -f "$home"
    rm -rf "$realhome"
    mkdir -p "$home"

    echo "== custom CANOPY_INSTALL_DIR with a space"
    run_raw CANOPY_INSTALL_BASE_URL="$base" CANOPY_INSTALL_DIR="$home/My Apps/canopy" "$SH" "$installer"
    check "custom dir install exits 0" [ "$status" = 0 ]
    check "custom dir Exec quoted" grep -qxF "Exec=\"$home/My Apps/canopy/bin/canopy\" %F" "$apps"
    run_raw "$SH" "$home/My Apps/canopy/uninstall.sh"
    check "custom dir uninstall" [ "$status" = 0 ]
    check "custom dir uninstall (2)" [ ! -e "$home/My Apps/canopy" ]
    check "custom dir uninstall (3)" [ ! -e "$apps" ]

    echo "== wget instead of curl"
    if command -v wget >/dev/null 2>&1 && wget --version 2>/dev/null | grep -q 'GNU Wget'; then
        path_without "$tmp/nocurl" curl
        run_path=$tmp/nocurl
        run_status
        check "install with wget exits 0" [ "$status" = 0 ]
        check "installed with wget" [ "$(cat "$opt/VERSION" 2>/dev/null)" = 103 ]
        run_status --uninstall
        run_path=/usr/local/bin:/usr/bin:/bin
    else
        echo "skipped: GNU wget is not installed"
    fi

    echo "== a download cut short runs nothing"
    size=$(wc -c <"$installer")
    for pct in 5 20 40 60 80 95 99; do
        rm -rf "$home"
        mkdir -p "$home"
        head -c $((size * pct / 100)) "$installer" >"$tmp/cut.sh"
        run_raw CANOPY_INSTALL_BASE_URL="$base" "$SH" "$tmp/cut.sh"
        check "cut at $pct%: nothing done" [ -z "$(ls -A "$home")" ]
        check "cut at $pct%: nothing said" [ "$(grep -E -c 'Installing|Removing|test mode' "$tmp/out" || true)" = 0 ]
    done
    rm -rf "$home"
}

# ---------------------------------------------------------------------------------------------
# The app-bundle logic (layout, bundle id, swap, symlink, uninstall). On Linux under a fake
# uname, which checks the logic, not BSD tools.
mac_flow() {
    if [ "$os" = Darwin ]; then
        arch=$(uname -m)
        if [ "$arch" = x86_64 ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ]; then arch=arm64; fi
        case $arch in
            arm64) target=aarch64-apple-darwin key=darwin-aarch64 ;;
            *) target=x86_64-apple-darwin key=darwin-x86_64 ;;
        esac
        base_path=/usr/local/bin:/usr/bin:/bin
    else
        mkdir -p "$tmp/fakemac"
        cat >"$tmp/fakemac/uname" <<'EOF'
#!/bin/sh
case ${1:-} in -m) echo arm64 ;; *) echo Darwin ;; esac
EOF
        chmod 755 "$tmp/fakemac/uname"
        target=aarch64-apple-darwin key=darwin-aarch64
        base_path=$tmp/fakemac:/usr/local/bin:/usr/bin:/bin
    fi
    run_path=$base_path
    home=$tmp/machome
    rm -rf "$home"
    mkdir -p "$home"
    mapp=$home/Applications/Canopy.app
    link=$home/.local/bin/canopy

    make_mac_release 102
    make_mac_release 103
    make_mac_release 106

    echo "== macOS: a failed first install leaves nothing"
    publish latest 106 0000000000000000000000000000000000000000000000000000000000000000
    run_status
    check "mac failed install exits non-zero" [ "$status" != 0 ]
    check "no ~/Applications left" [ ! -e "$home/Applications" ]

    echo "== macOS: install, upgrade, refusals"
    publish latest 102
    run_status
    check "mac install exits 0" [ "$status" = 0 ]
    show
    check "app bundle installed" [ -x "$mapp/Contents/MacOS/canopy" ]
    check "mac command symlink" [ "$(readlink "$link")" = "$mapp/Contents/MacOS/canopy" ]
    check "mac command runs" [ "$("$link")" = "canopy 102 (fake mac)" ]
    publish latest 103
    run_status
    check "mac upgrade exits 0" [ "$status" = 0 ]
    check "mac says updated" out_has "Updated Canopy build 102 -> build 103"
    check "mac runs 103" [ "$("$link")" = "canopy 103 (fake mac)" ]
    check "no temporary folders left in ~/Applications" [ "$(ls -A "$home/Applications")" = Canopy.app ]
    publish latest 102
    run_status
    check "mac does not downgrade silently" [ "$status" != 0 ]
    check "mac does not downgrade silently (2)" [ "$("$link")" = "canopy 103 (fake mac)" ]
    publish latest 106 0000000000000000000000000000000000000000000000000000000000000000
    run_status
    check "mac mismatch refused" [ "$status" != 0 ]
    check "mac old version kept" [ "$("$link")" = "canopy 103 (fake mac)" ]

    echo "== macOS: Ctrl+C after the new app is moved in"
    publish latest 106
    stub_mv "$tmp/mv-mac" dst "$mapp" interrupt
    run_path=$tmp/mv-mac:$base_path
    run_status
    run_path=$base_path
    check "interrupted mac install exits 130" [ "$status" = 130 ]
    check "mac old version put back" [ "$("$link")" = "canopy 103 (fake mac)" ]
    check "no temporary folders left in ~/Applications" [ "$(ls -A "$home/Applications")" = Canopy.app ]
    publish latest 103

    echo "== macOS: uninstall"
    mkdir -p "$home/Other.app/Contents"
    sed 's/io.necturalabs.Canopy/com.example.Other/' "$mapp/Contents/Info.plist" >"$home/Other.app/Contents/Info.plist"
    run_raw CANOPY_INSTALL_BASE_URL="$base" CANOPY_INSTALL_DIR="$home/Other.app" "$SH" "$installer" --uninstall
    check "another app is not removed" [ "$status" != 0 ]
    check "another app is not removed (2)" [ -d "$home/Other.app" ]
    mkdir -p "$home/Library/Caches/io.necturalabs.Canopy" "$home/Library/Logs/Canopy"
    echo "cache" >"$home/Library/Caches/io.necturalabs.Canopy/c"
    echo "someone else's" >"$home/Library/Logs/Canopy/x.log"
    run_status --uninstall --purge --yes
    check "mac uninstall exits 0" [ "$status" = 0 ]
    show
    check "app bundle removed" [ ! -e "$mapp" ]
    check "mac symlink removed" [ ! -e "$link" ]
    check "mac symlink removed (2)" [ ! -L "$link" ]
    check "vendor-named cache purged" [ ! -e "$home/Library/Caches/io.necturalabs.Canopy" ]
    check "unmarked Logs/Canopy kept" [ -f "$home/Library/Logs/Canopy/x.log" ]
}

if [ "$os" = Linux ]; then linux_flow; fi
mac_flow

echo
if [ "$failures" -ne 0 ]; then
    echo "$failures of $checks check(s) failed (shell: $sh_cmd). Last installer output:"
    cat "$tmp/out"
    exit 1
fi
echo "All $checks installer checks passed (shell: $sh_cmd)."
