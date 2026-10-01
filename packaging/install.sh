#!/bin/sh
# canopy-installer (this marker line identifies the script; keep it)
#
# Installs, upgrades or removes Canopy for the current user on Linux and macOS, without sudo:
#
#   curl -fsSL https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/install.sh | sh -s -- --uninstall
#
# Run with --help for the options. The script reads the update manifest in NecturaLabs/app-updates
# (canopy/latest.json, or canopy/beta.json), downloads this platform's archive over HTTPS only,
# checks its SHA-256 against the manifest and refuses anything that does not match. It never runs
# what it downloads, never asks for sudo and never installs git.
#
# Layout (Linux): everything lives in ~/.local/opt/canopy (CANOPY_INSTALL_DIR overrides it):
#   bin/canopy  share/applications/  share/icons/  share/metainfo/  legal/  LICENSE  README.md
#   VERSION  uninstall.sh  .canopy-install (the receipt: what was created outside the folder)
# Outside it, only: ~/.local/bin/canopy (a symlink; --no-path skips it), and the desktop entry and
# icons under ${XDG_DATA_HOME:-~/.local/share}/{applications,icons/hicolor}.
# Layout (macOS): ~/Applications/Canopy.app (CANOPY_INSTALL_DIR names the .app), and the
# ~/.local/bin/canopy symlink unless --no-path.
#
# Canopy's own settings and data (~/.local/share/canopy and friends) are never touched, except by
# --uninstall --purge.
#
# Maintainers: packaging/README.md. The source is packaging/install.sh in NecturaLabs/Canopy;
# the release workflow publishes it to NecturaLabs/app-updates/canopy/install.sh.
set -eu

APP_ID=io.necturalabs.Canopy
FEED_URL=https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy
RELEASES_URL=https://github.com/NecturaLabs/app-updates/releases/download
MIN_GLIBC=2.35

# ---------------------------------------------------------------------------------------------
# Canopy's user data, removed only by --purge. Keep this list in step with the app (and with
# packaging/install.ps1). Old (eframe::storage_dir) and current vendor-specific locations.
purge_targets() {
    case $OS in
        linux)
            printf '%s\n' \
                "${XDG_DATA_HOME:-$HOME/.local/share}/canopy" \
                "${XDG_CONFIG_HOME:-$HOME/.config}/canopy" \
                "${XDG_STATE_HOME:-$HOME/.local/state}/canopy" \
                "${XDG_CACHE_HOME:-$HOME/.cache}/canopy"
            ;;
        darwin)
            printf '%s\n' \
                "$HOME/Library/Application Support/canopy" \
                "$HOME/Library/Application Support/$APP_ID" \
                "$HOME/Library/Logs/Canopy" \
                "$HOME/Library/Caches/$APP_ID"
            ;;
    esac
}
# ---------------------------------------------------------------------------------------------

usage() {
    cat <<'EOF'
Install Canopy for the current user (no sudo).

Usage:
  curl -fsSL https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/install.sh | sh -s -- [options]
  sh install.sh [options]

Options:
  --beta           Install the newest build, including pre-releases (the beta channel).
  --version X      Install version X (for example 0.2.0) instead of the newest one.
  --no-path        Do not create the ~/.local/bin/canopy symlink.
  --uninstall      Remove Canopy: exactly the files this script created. Settings and data stay.
  --purge          With --uninstall: also delete Canopy's settings, logs and caches (asks first).
  -y, --yes        Do not ask for confirmation (for --purge).
  -h, --help       Show this help.

Environment:
  CANOPY_INSTALL_DIR   Install folder. Linux: default ~/.local/opt/canopy.
                       macOS: the app bundle, default ~/Applications/Canopy.app.
  XDG_DATA_HOME        Where the desktop entry and icons are registered (Linux).

Running the installed copy of this script, ~/.local/opt/canopy/uninstall.sh, uninstalls.
Re-running the installer upgrades in place; a failed upgrade keeps the installed version.
EOF
}

say() { printf '%s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}
have() { command -v "$1" >/dev/null 2>&1; }

# ---------------------------------------------------------------------------------------------
# Options

MODE=install
CHANNEL=latest
PIN=
LINK=1
PURGE=0
YES=0
SELF_DIR=
case ${0##*/} in
    uninstall.sh)
        MODE=uninstall
        SELF_DIR=$(cd "$(dirname "$0")" && pwd -P)
        ;;
esac

while [ $# -gt 0 ]; do
    case $1 in
        --beta) CHANNEL=beta ;;
        --version)
            [ $# -ge 2 ] || die "--version needs a value, for example --version 0.2.0"
            PIN=$2
            shift
            ;;
        --version=*) PIN=${1#--version=} ;;
        --no-path) LINK=0 ;;
        --uninstall) MODE=uninstall ;;
        --purge) PURGE=1 ;;
        -y | --yes) YES=1 ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            printf 'error: unknown option: %s (see --help)\n' "$1" >&2
            exit 2
            ;;
    esac
    shift
done
if [ "$PURGE" = 1 ] && [ "$MODE" != uninstall ]; then
    die "--purge only goes with --uninstall"
fi
PIN=${PIN#v}

# Test-only: tools and CI point the script at a fake feed on this machine. Only plain-HTTP
# loopback URLs are accepted, so this can never redirect a real install anywhere else.
INSECURE_LOCAL=0
if [ -n "${CANOPY_INSTALL_BASE_URL:-}" ]; then
    case $CANOPY_INSTALL_BASE_URL in
        http://127.0.0.1:[0-9]* | http://localhost:[0-9]*) ;;
        *) die "CANOPY_INSTALL_BASE_URL is for tests and only accepts http://127.0.0.1:<port> or http://localhost:<port>" ;;
    esac
    base=${CANOPY_INSTALL_BASE_URL%/}
    FEED_URL=$base/canopy
    RELEASES_URL=$base/releases/download
    INSECURE_LOCAL=1
    warn "test mode: using the feed at $base"
fi

# ---------------------------------------------------------------------------------------------
# Platform

[ -n "${HOME:-}" ] || die "HOME is not set"
case $(uname -s) in
    Linux) OS=linux ;;
    Darwin) OS=darwin ;;
    MINGW* | MSYS* | CYGWIN* | Windows_NT)
        die "on Windows, run this in PowerShell instead:
  irm https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/install.ps1 | iex"
        ;;
    *) die "Canopy has no build for $(uname -s); it runs on Linux, macOS and Windows" ;;
esac
case $(uname -m) in
    x86_64 | amd64) ARCH=x86_64 ;;
    aarch64 | arm64) ARCH=aarch64 ;;
    *) die "Canopy has no build for $(uname -m) processors yet (only x86_64 and arm64)" ;;
esac
# An x86_64 shell under Rosetta on an Apple silicon Mac should still get the native build.
if [ "$OS" = darwin ] && [ "$ARCH" = x86_64 ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ]; then
    ARCH=aarch64
fi
KEY=$OS-$ARCH
case $KEY in
    linux-x86_64) TARGET=x86_64-unknown-linux-gnu ;;
    linux-aarch64) TARGET=aarch64-unknown-linux-gnu ;;
    darwin-x86_64) TARGET=x86_64-apple-darwin ;;
    darwin-aarch64) TARGET=aarch64-apple-darwin ;;
esac
case $OS in
    linux) PRETTY="Linux $ARCH" ;;
    darwin) PRETTY="macOS $ARCH" ;;
esac

# Paths with these characters cannot be written safely into a desktop entry's Exec= line or the
# receipt, so they are refused rather than mangled.
check_path() {
    case $1 in
        *'
'* | *'"'* | *'`'* | *'$'* | *\\* | *'%'*)
            die "unsupported character in path: $1 (choose a folder with CANOPY_INSTALL_DIR)"
            ;;
    esac
}

DATA_HOME=${XDG_DATA_HOME:-$HOME/.local/share}
BIN_DIR=$HOME/.local/bin
if [ "$OS" = linux ]; then
    # The installed uninstall.sh always removes the folder it sits in.
    ROOT=${SELF_DIR:-${CANOPY_INSTALL_DIR:-$HOME/.local/opt/canopy}}
else
    ROOT=${CANOPY_INSTALL_DIR:-$HOME/Applications/Canopy.app}
    case $ROOT in
        *.app) ;;
        *) die "on macOS, CANOPY_INSTALL_DIR names the app bundle and must end in .app" ;;
    esac
fi
case $ROOT in
    /*) ;;
    *) ROOT=$(pwd -P)/$ROOT ;;
esac
ROOT=${ROOT%/}
for p in "$ROOT" "$HOME" "$DATA_HOME"; do check_path "$p"; done
case $ROOT in
    "" | / | "$HOME" | "$HOME/.local" | "$HOME/.local/opt" | "$HOME/.local/bin" | "$HOME/.local/share" | "$HOME/Applications" | /Applications | /usr | /usr/* | /opt | /bin | /etc)
        die "refusing to use $ROOT as Canopy's own folder; it must be a folder for Canopy alone"
        ;;
esac
LINK_PATH=$BIN_DIR/canopy
if [ "$OS" = linux ]; then
    APP_BIN=$ROOT/bin/canopy
else
    APP_BIN=$ROOT/Contents/MacOS/canopy
fi

# ---------------------------------------------------------------------------------------------
# Helpers

# Downloads $1 to $2. HTTPS with TLS 1.2 or newer only, also across redirects; plain HTTP only to
# the loopback test feed.
fetch() {
    case $1 in
        https://*) ;;
        http://127.0.0.1:* | http://localhost:*)
            [ "$INSECURE_LOCAL" = 1 ] || die "refusing to download over plain HTTP: $1"
            ;;
        *) die "refusing to download from $1" ;;
    esac
    if have curl; then
        if [ "$INSECURE_LOCAL" = 1 ]; then
            curl --fail --silent --show-error --location --proto '=http' --proto-redir '=http' \
                --noproxy '127.0.0.1,localhost' --output "$2" "$1"
        else
            curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
                --tlsv1.2 --retry 3 --connect-timeout 20 --output "$2" "$1"
        fi
    elif have wget && wget --version 2>/dev/null | grep -q 'GNU Wget'; then
        if [ "$INSECURE_LOCAL" = 1 ]; then
            wget --quiet --no-proxy --output-document="$2" "$1"
        else
            wget --quiet --https-only --secure-protocol=TLSv1_2 --tries=3 --timeout=20 \
                --output-document="$2" "$1"
        fi
    else
        die "need curl or GNU wget to download Canopy"
    fi
}

sha256_of() {
    if have sha256sum; then
        sha256sum -- "$1" | awk '{print $1}'
    elif have shasum; then
        shasum -a 256 -- "$1" | awk '{print $1}'
    elif have openssl; then
        openssl dgst -sha256 -r "$1" | awk '{print $1}'
    else
        die "need sha256sum, shasum or openssl to verify the download"
    fi
}

# Flattens a JSON document into "path<TAB>value" lines (object keys joined with ".", array
# items by index), enough to read the manifest without jq or python.
json_flat() {
    awk '
    function path() {
        if (d == 0) return ""
        if (kind[d] == "{") return (pre[d] == "" ? "" : pre[d] ".") key[d]
        return (pre[d] == "" ? "" : pre[d] ".") idx[d]
    }
    BEGIN { RS = "\001" }
    {
        s = $0; n = length(s); i = 1; d = 0
        while (i <= n) {
            c = substr(s, i, 1)
            if (c == "{" || c == "[") {
                p = path(); d++; kind[d] = c; pre[d] = p; idx[d] = 0; want[d] = (c == "{"); i++
            } else if (c == "}" || c == "]") {
                d--; i++
            } else if (c == ",") {
                if (kind[d] == "{") want[d] = 1; else idx[d]++
                i++
            } else if (c == ":") {
                want[d] = 0; i++
            } else if (c == "\"") {
                v = ""; j = i + 1
                while (j <= n) {
                    ch = substr(s, j, 1)
                    if (ch == "\\") {
                        nx = substr(s, j + 1, 1)
                        if (nx == "u") { v = v "?"; j += 6; continue }
                        if (nx == "n" || nx == "t" || nx == "r") nx = " "
                        v = v nx; j += 2; continue
                    }
                    if (ch == "\"") break
                    if (ch == "\t" || ch == "\n" || ch == "\r") ch = " "
                    v = v ch; j++
                }
                i = j + 1
                if (kind[d] == "{" && want[d]) key[d] = v
                else print path() "\t" v
            } else if (c ~ /[ \t\r\n]/) {
                i++
            } else {
                j = i
                while (j <= n && substr(s, j, 1) !~ /[],} \t\r\n]/) j++
                print path() "\t" substr(s, i, j - i)
                i = j
            }
        }
    }' "$1"
}

json_get() { awk -F '\t' -v k="$2" '$1 == k { print $2; exit }' "$1"; }

# Exit 0 when version $1 is newer than $2 (semantic versioning, pre-releases below releases).
version_gt() {
    awk -v a="$1" -v b="$2" '
    function isnum(x) { return x ~ /^[0-9]+$/ }
    function cmpid(x, y) {
        if (isnum(x) && isnum(y)) return (x + 0 > y + 0) - (x + 0 < y + 0)
        if (isnum(x)) return -1
        if (isnum(y)) return 1
        return (x > y) - (x < y)
    }
    function cmp(x, y,    xc, yc, xp, yp, xs, ys, nx, ny, k, r) {
        xc = x; xp = ""; if (index(x, "-")) { xc = substr(x, 1, index(x, "-") - 1); xp = substr(x, index(x, "-") + 1) }
        yc = y; yp = ""; if (index(y, "-")) { yc = substr(y, 1, index(y, "-") - 1); yp = substr(y, index(y, "-") + 1) }
        nx = split(xc, xs, "."); ny = split(yc, ys, ".")
        for (k = 1; k <= (nx > ny ? nx : ny); k++) {
            r = (xs[k] + 0 > ys[k] + 0) - (xs[k] + 0 < ys[k] + 0)
            if (r) return r
        }
        if (xp == yp) return 0
        if (xp == "") return 1
        if (yp == "") return -1
        nx = split(xp, xs, "."); ny = split(yp, ys, ".")
        for (k = 1; k <= nx && k <= ny; k++) { r = cmpid(xs[k], ys[k]); if (r) return r }
        return (nx > ny) - (nx < ny)
    }
    BEGIN { exit !(cmp(a, b) > 0) }'
}

valid_version() {
    printf '%s\n' "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$'
}

# Reads the manifest for channel $1 into $WORK/$1.flat; returns 1 when the channel has none.
read_channel() {
    if ! fetch "$FEED_URL/$1.json" "$WORK/$1.json" 2>"$WORK/$1.err"; then
        if [ "$1" = beta ]; then return 1; fi
        cat "$WORK/$1.err" >&2
        die "could not read the update manifest $FEED_URL/$1.json"
    fi
    json_flat "$WORK/$1.json" >"$WORK/$1.flat"
    v=$(json_get "$WORK/$1.flat" version)
    valid_version "$v" || die "the update manifest $1.json names no valid version"
}

confirm() {
    [ "$YES" = 1 ] && return 0
    if ! (: </dev/tty) 2>/dev/null; then
        die "$1: no terminal to ask on; re-run with --yes to confirm"
    fi
    printf '%s [y/N] ' "$1" >/dev/tty
    read -r answer </dev/tty || answer=
    case $answer in y | Y | yes | YES | Yes) return 0 ;; esac
    return 1
}

refresh_desktop_caches() {
    [ "$OS" = linux ] || return 0
    if have update-desktop-database && [ -d "$DATA_HOME/applications" ]; then
        update-desktop-database -q "$DATA_HOME/applications" 2>/dev/null || true
    fi
    # Only refresh an icon cache that already exists: creating one in the user's hicolor folder
    # would leave a file behind that this script did not need.
    if have gtk-update-icon-cache && [ -f "$DATA_HOME/icons/hicolor/icon-theme.cache" ]; then
        gtk-update-icon-cache -q -t -f "$DATA_HOME/icons/hicolor" 2>/dev/null || true
    elif [ -d "$DATA_HOME/icons/hicolor" ]; then
        touch "$DATA_HOME/icons/hicolor" 2>/dev/null || true
    fi
}

# Whether $1 is a symlink pointing at $2.
links_to() { [ -L "$1" ] && [ "$(readlink "$1")" = "$2" ]; }

git_report() {
    say ""
    if have git; then
        say "git:      $(git --version 2>/dev/null || echo 'found')"
    else
        say "git:      NOT FOUND. Canopy needs git 2.30 or newer on PATH."
    fi
    if have git && git lfs version >/dev/null 2>&1; then
        say "git-lfs:  $(git lfs version 2>/dev/null | head -n 1)"
    else
        say "git-lfs:  not found (only needed for repositories that use Git LFS)"
    fi
    if ! have git || ! git lfs version >/dev/null 2>&1; then
        hint=
        if [ "$OS" = darwin ]; then
            hint="xcode-select --install   (git), or: brew install git git-lfs"
        elif [ -r /etc/os-release ]; then
            # shellcheck disable=SC1091
            ids=$(. /etc/os-release && printf '%s %s' "${ID:-}" "${ID_LIKE:-}")
            case " $ids " in
                *" debian "* | *" ubuntu "*) hint="sudo apt install git git-lfs" ;;
                *" fedora "* | *" rhel "* | *" centos "*) hint="sudo dnf install git git-lfs" ;;
                *" arch "*) hint="sudo pacman -S git git-lfs" ;;
                *" opensuse"* | *" suse "*) hint="sudo zypper install git git-lfs" ;;
                *" alpine "*) hint="sudo apk add git git-lfs" ;;
            esac
        fi
        say "          Install with your package manager${hint:+: $hint}"
        say "          (this installer never installs git for you)."
    fi
}

# ---------------------------------------------------------------------------------------------
# Uninstall

uninstall_linux() {
    receipt=$ROOT/.canopy-install
    if [ ! -e "$ROOT" ]; then
        say "Canopy is not installed in $ROOT; nothing to remove."
        return 0
    fi
    [ -f "$receipt" ] || die "$ROOT has no .canopy-install receipt, so this script did not create it; not touching it"
    say "Removing Canopy from $ROOT"
    while IFS= read -r line; do
        case $line in
            link=*)
                f=${line#link=}
                if links_to "$f" "$ROOT/bin/canopy"; then rm -f -- "$f" && say "  removed $f"; fi
                ;;
            desktop=*)
                f=${line#desktop=}
                if [ -f "$f" ] && grep -qxF "X-Canopy-Install-Dir=$ROOT" "$f"; then
                    rm -f -- "$f" && say "  removed $f"
                fi
                ;;
            icon=*)
                # icon=<registered copy>|<original inside the install folder>
                pair=${line#icon=}
                f=${pair%%|*}
                src=${pair#*|}
                if [ -f "$f" ] && cmp -s -- "$f" "$src"; then rm -f -- "$f" && say "  removed $f"; fi
                ;;
        esac
    done <"$receipt"
    rm -rf -- "$ROOT"
    say "  removed $ROOT"
    refresh_desktop_caches
}

uninstall_darwin() {
    if [ -e "$ROOT" ]; then
        id=$(bundle_id "$ROOT")
        [ "$id" = "$APP_ID" ] || die "$ROOT is not Canopy (bundle id '${id:-none}'); not touching it"
        rm -rf -- "$ROOT"
        say "Removed $ROOT"
    else
        say "Canopy is not installed at $ROOT."
    fi
    if links_to "$LINK_PATH" "$APP_BIN"; then rm -f -- "$LINK_PATH" && say "Removed $LINK_PATH"; fi
}

bundle_id() {
    plist=$1/Contents/Info.plist
    [ -f "$plist" ] || return 0
    awk '/<key>CFBundleIdentifier<\/key>/ { getline; gsub(/.*<string>|<\/string>.*/, ""); print; exit }' "$plist"
}

# Asks, before anything is removed, whether to delete the user data --purge would delete.
# Sets PURGE_OK=1 when there is something to delete and the user agreed.
confirm_purge() {
    PURGE_OK=0
    existing=$(purge_targets | while IFS= read -r d; do
        if [ -e "$d" ]; then printf '  %s\n' "$d"; fi
    done)
    if [ -z "$existing" ]; then
        say "No Canopy settings or data found to purge."
        return 0
    fi
    say "--purge deletes Canopy's settings, logs and caches:"
    say "$existing"
    if confirm "Delete them?"; then
        PURGE_OK=1
    else
        say "Keeping them."
    fi
}

purge() {
    if [ "$PURGE_OK" = 1 ]; then
        purge_targets | while IFS= read -r d; do
            if [ -e "$d" ]; then rm -rf -- "$d" && say "  removed $d"; fi
        done
    fi
    say ""
    say "Tokens saved in Settings -> Accounts stay in the system keychain (service"
    say "\"$APP_ID\"): remove them in Canopy before uninstalling, or with your keychain app."
}

# ---------------------------------------------------------------------------------------------
# Install

LOCK=
WORK=
CREATED_ROOT=0
cleanup() {
    [ -n "$WORK" ] && rm -rf -- "$WORK"
    [ -n "$LOCK" ] && rmdir -- "$LOCK" 2>/dev/null
    # A first install that failed leaves nothing behind but its own receipt.
    if [ "$CREATED_ROOT" = 1 ] && [ -d "$ROOT" ] && only_receipt_in "$ROOT"; then
        rm -rf -- "$ROOT"
    fi
    return 0
}

only_receipt_in() {
    for e in "$1"/* "$1"/.[!.]* "$1"/..?*; do
        [ -e "$e" ] || [ -L "$e" ] || continue
        [ "$e" = "$1/.canopy-install" ] || return 1
    done
    return 0
}

# Finds the release: sets VERSION, ASSET, ASSET_URL and ASSET_SHA.
resolve_release() {
    ext=tar.gz
    if [ -n "$PIN" ]; then
        valid_version "$PIN" || die "--version $PIN is not a version like 0.2.0 or 0.3.0-beta.1"
        VERSION=$PIN
    fi
    say "Reading the update manifest"
    read_channel latest
    VERSION_LATEST=$(json_get "$WORK/latest.flat" version)
    use=latest
    if [ "$CHANNEL" = beta ] || [ -n "$PIN" ]; then
        if read_channel beta; then
            v=$(json_get "$WORK/beta.flat" version)
            if [ -n "$PIN" ]; then
                [ "$v" = "$PIN" ] && use=beta
            elif version_gt "$v" "$VERSION_LATEST"; then
                use=beta
            fi
        fi
    fi
    mv=$(json_get "$WORK/$use.flat" version)
    if [ -z "$PIN" ] || [ "$PIN" = "$mv" ]; then
        VERSION=$mv
        ASSET_URL=$(json_get "$WORK/$use.flat" "platforms.$KEY.url")
        ASSET_SHA=$(json_get "$WORK/$use.flat" "platforms.$KEY.sha256")
        if [ -z "$ASSET_URL" ]; then
            keys=$(awk -F '\t' '$1 ~ /^platforms\.[^.]*\.url$/ { sub(/^platforms\./, "", $1); sub(/\.url$/, "", $1); printf "%s ", $1 }' "$WORK/$use.flat")
            die "Canopy $VERSION has no build for $PRETTY ($KEY) yet. Builds in this release: ${keys:-none}"
        fi
        ASSET=${ASSET_URL##*/}
    else
        # An older or other version: its release's SHA256SUMS gives the checksum.
        ASSET=canopy-$VERSION-$TARGET.$ext
        say "Reading the checksums of Canopy $VERSION"
        fetch "$RELEASES_URL/canopy-v$VERSION/SHA256SUMS" "$WORK/SHA256SUMS" 2>"$WORK/sums.err" ||
            die "Canopy $VERSION is not published (no $RELEASES_URL/canopy-v$VERSION/SHA256SUMS)"
        ASSET_SHA=$(awk -v f="$ASSET" '{ n = $2; sub(/^\*/, "", n) } n == f { print $1; exit }' "$WORK/SHA256SUMS")
        [ -n "$ASSET_SHA" ] || die "Canopy $VERSION has no build for $PRETTY ($KEY)"
        ASSET_URL=$RELEASES_URL/canopy-v$VERSION/$ASSET
    fi
    # Only ever download Canopy's own archive from Canopy's own release in the feed repository.
    [ "$ASSET_URL" = "$RELEASES_URL/canopy-v$VERSION/$ASSET" ] ||
        die "the manifest points outside Canopy's releases: $ASSET_URL"
    [ "$ASSET" = "canopy-$VERSION-$TARGET.$ext" ] ||
        die "unexpected archive name in the manifest: $ASSET"
    printf '%s\n' "$ASSET_SHA" | grep -Eq '^[0-9a-f]{64}$' ||
        die "the manifest has no valid sha256 for $ASSET"
}

download_and_verify() {
    say "Downloading $ASSET_URL"
    fetch "$ASSET_URL" "$WORK/$ASSET" || die "download failed: $ASSET_URL"
    got=$(sha256_of "$WORK/$ASSET")
    if [ "$got" != "$ASSET_SHA" ]; then
        die "checksum mismatch for $ASSET
  expected $ASSET_SHA
  got      $got
Nothing was installed. Try again later; if it persists, report it."
    fi
    say "Checksum verified (sha256 $got)"
    mkdir -p "$WORK/x"
    tar -xzf "$WORK/$ASSET" -C "$WORK/x" || die "could not unpack $ASSET"
    SRC=$WORK/x/canopy-$VERSION-$TARGET
    [ -d "$SRC" ] || die "$ASSET does not contain canopy-$VERSION-$TARGET/"
}

# Builds the new install folder's content in $1 from the unpacked archive.
stage_linux() {
    new=$1
    [ -f "$SRC/canopy" ] || die "$ASSET has no canopy binary"
    mkdir -p "$new/bin" "$new/share/applications" "$new/share/metainfo" \
        "$new/share/icons/hicolor/256x256/apps" "$new/share/icons/hicolor/scalable/apps"
    cp "$SRC/canopy" "$new/bin/canopy"
    chmod 755 "$new/bin/canopy"
    for f in README.md LICENSE; do
        if [ -f "$SRC/$f" ]; then cp "$SRC/$f" "$new/$f"; fi
    done
    if [ -d "$SRC/legal" ]; then cp -R "$SRC/legal" "$new/legal"; fi
    if [ -f "$SRC/$APP_ID.desktop" ]; then
        awk -v ex="Exec=\"$ROOT/bin/canopy\" %F" -v marker="X-Canopy-Install-Dir=$ROOT" '
            /^Exec=/ { print ex; next }
            /^X-Canopy-Install-Dir=/ { next }
            { print }
            END { print marker }' "$SRC/$APP_ID.desktop" >"$new/share/applications/$APP_ID.desktop"
    fi
    if [ -f "$SRC/$APP_ID.metainfo.xml" ]; then cp "$SRC/$APP_ID.metainfo.xml" "$new/share/metainfo/"; fi
    if [ -f "$SRC/$APP_ID.png" ]; then cp "$SRC/$APP_ID.png" "$new/share/icons/hicolor/256x256/apps/"; fi
    if [ -f "$SRC/$APP_ID.svg" ]; then cp "$SRC/$APP_ID.svg" "$new/share/icons/hicolor/scalable/apps/"; fi
    printf '%s\n' "$VERSION" >"$new/VERSION"
    install_self "$new/uninstall.sh"
}

# Copies this script (the uninstaller is the installer run as uninstall.sh) to $1.
install_self() {
    if [ -f "$0" ] && sed -n 2p "$0" 2>/dev/null | grep -q '^# canopy-installer'; then
        cp "$0" "$1"
    elif fetch "$FEED_URL/install.sh" "$1" 2>/dev/null &&
        sed -n 2p "$1" | grep -q '^# canopy-installer' && sh -n "$1" 2>/dev/null; then
        : # Piped into sh: the published copy of this script, fetched again.
    else
        warn "could not save uninstall.sh; uninstall with the one-line installer and --uninstall"
        rm -f -- "$1"
        return 0
    fi
    chmod 755 "$1"
}

# Moves the staged content ($1) into $ROOT, replacing the old version. On any failure the old
# version is put back.
swap_in() {
    new=$1
    old=$(mktemp -d "$ROOT/.previous.XXXXXX")
    for e in "$ROOT"/* ; do
        [ -e "$e" ] || [ -L "$e" ] || continue
        mv -- "$e" "$old/" || { restore "$old"; return 1; }
    done
    for e in "$new"/* ; do
        [ -e "$e" ] || [ -L "$e" ] || continue
        mv -- "$e" "$ROOT/" || { restore "$old"; return 1; }
    done
    rm -rf -- "$old"
}

restore() {
    for e in "$ROOT"/* ; do
        [ -e "$e" ] || [ -L "$e" ] || continue
        rm -rf -- "$e"
    done
    for e in "$1"/* ; do
        [ -e "$e" ] || [ -L "$e" ] || continue
        mv -- "$e" "$ROOT/"
    done
    rmdir -- "$1" 2>/dev/null || true
}

# Registers the desktop entry, icons and command outside the install folder; writes the receipt.
integrate_linux() {
    receipt=$ROOT/.canopy-install
    tmp_receipt=$ROOT/.canopy-install.new
    {
        printf '# Written by the Canopy installer: what it created outside %s.\n' "$ROOT"
        printf 'version=%s\n' "$VERSION"
    } >"$tmp_receipt"

    apps=$DATA_HOME/applications
    if [ -f "$ROOT/share/applications/$APP_ID.desktop" ]; then
        mkdir -p "$apps"
        cp "$ROOT/share/applications/$APP_ID.desktop" "$apps/$APP_ID.desktop"
        printf 'desktop=%s\n' "$apps/$APP_ID.desktop" >>"$tmp_receipt"
    fi
    for rel in 256x256/apps/$APP_ID.png scalable/apps/$APP_ID.svg; do
        src=$ROOT/share/icons/hicolor/$rel
        [ -f "$src" ] || continue
        dst=$DATA_HOME/icons/hicolor/$rel
        mkdir -p "$(dirname "$dst")"
        cp "$src" "$dst"
        printf 'icon=%s|%s\n' "$dst" "$src" >>"$tmp_receipt"
    done
    link_command "$ROOT/bin/canopy" "$tmp_receipt"
    mv -f -- "$tmp_receipt" "$receipt"
    refresh_desktop_caches
}

# Creates or removes the ~/.local/bin/canopy symlink to $1; records it in receipt $2 if given.
link_command() {
    if [ "$LINK" = 1 ]; then
        if { [ -e "$LINK_PATH" ] || [ -L "$LINK_PATH" ]; } && ! links_to "$LINK_PATH" "$1"; then
            warn "$LINK_PATH already exists and is not this installer's link; left it alone (remove it, then re-run, to get the 'canopy' command)"
            return 0
        fi
        mkdir -p "$BIN_DIR"
        ln -sf -- "$1" "$LINK_PATH"
        if [ -n "${2:-}" ]; then printf 'link=%s\n' "$LINK_PATH" >>"$2"; fi
        case ":${PATH:-}:" in
            *":$BIN_DIR:"*) ;;
            *) PATH_HINT=1 ;;
        esac
    elif links_to "$LINK_PATH" "$1"; then
        rm -f -- "$LINK_PATH"
        say "Removed $LINK_PATH (--no-path)"
    fi
}

install_linux() {
    if [ -e "$ROOT" ] && [ ! -d "$ROOT" ]; then die "$ROOT exists and is not a folder"; fi
    if [ -d "$ROOT" ] && [ ! -f "$ROOT/.canopy-install" ] && [ -n "$(ls -A "$ROOT")" ]; then
        die "$ROOT is not empty and was not created by this installer; choose another CANOPY_INSTALL_DIR"
    fi
    if [ ! -d "$ROOT" ]; then
        mkdir -p "$ROOT"
        CREATED_ROOT=1
    fi
    LOCK=$ROOT/.lock
    mkdir "$LOCK" 2>/dev/null || {
        LOCK=
        die "another Canopy install is running (remove $ROOT/.lock if it is not)"
    }
    if [ ! -f "$ROOT/.canopy-install" ]; then
        printf '# Written by the Canopy installer.\n' >"$ROOT/.canopy-install"
    fi
    WORK=$(mktemp -d "$ROOT/.staging.XXXXXX")
    old_version=
    if [ -f "$ROOT/VERSION" ]; then old_version=$(cat "$ROOT/VERSION"); fi

    resolve_release
    download_and_verify
    mkdir "$WORK/new"
    stage_linux "$WORK/new"
    swap_in "$WORK/new" || die "could not move the new version into $ROOT; the previous one was kept"
    CREATED_ROOT=0
    integrate_linux

    say ""
    if [ -z "$old_version" ]; then
        say "Installed Canopy $VERSION in $ROOT"
    elif [ "$old_version" = "$VERSION" ]; then
        say "Reinstalled Canopy $VERSION in $ROOT"
    else
        say "Updated Canopy $old_version -> $VERSION in $ROOT"
    fi
    if [ "$LINK" = 1 ] && links_to "$LINK_PATH" "$ROOT/bin/canopy"; then
        say "Command:  $LINK_PATH -> $ROOT/bin/canopy"
    fi
    if [ -f "$DATA_HOME/applications/$APP_ID.desktop" ]; then
        say "Launcher: $DATA_HOME/applications/$APP_ID.desktop"
    fi
    say "Remove:   $ROOT/uninstall.sh"
}

install_darwin() {
    parent=$(dirname "$ROOT")
    if [ -e "$ROOT" ]; then
        id=$(bundle_id "$ROOT")
        [ "$id" = "$APP_ID" ] || die "$ROOT exists and is not Canopy (bundle id '${id:-none}'); not replacing it"
    fi
    mkdir -p "$parent"
    LOCK=$parent/.Canopy-install.lock
    mkdir "$LOCK" 2>/dev/null || {
        LOCK=
        die "another Canopy install is running (remove $parent/.Canopy-install.lock if it is not)"
    }
    WORK=$(mktemp -d "$parent/.Canopy-install.XXXXXX")
    old_version=
    if [ -f "$ROOT/Contents/Info.plist" ]; then
        old_version=$(awk '/<key>CFBundleShortVersionString<\/key>/ { getline; gsub(/.*<string>|<\/string>.*/, ""); print; exit }' "$ROOT/Contents/Info.plist")
    fi

    resolve_release
    download_and_verify
    [ -x "$SRC/Canopy.app/Contents/MacOS/canopy" ] || die "$ASSET has no Canopy.app"
    [ "$(bundle_id "$SRC/Canopy.app")" = "$APP_ID" ] || die "$ASSET holds an app with the wrong bundle id"
    mv -- "$SRC/Canopy.app" "$WORK/Canopy.app"
    if [ -e "$ROOT" ]; then
        mv -- "$ROOT" "$WORK/previous.app" || die "could not move the old $ROOT aside; nothing changed"
        if ! mv -- "$WORK/Canopy.app" "$ROOT"; then
            mv -- "$WORK/previous.app" "$ROOT"
            die "could not install the new version; the previous one was kept"
        fi
    else
        mv -- "$WORK/Canopy.app" "$ROOT" || die "could not create $ROOT"
    fi
    link_command "$APP_BIN" ""

    say ""
    if [ -z "$old_version" ]; then
        say "Installed Canopy $VERSION as $ROOT"
    elif [ "$old_version" = "$VERSION" ]; then
        say "Reinstalled Canopy $VERSION as $ROOT"
    else
        say "Updated Canopy $old_version -> $VERSION at $ROOT"
    fi
    if [ "$LINK" = 1 ] && links_to "$LINK_PATH" "$APP_BIN"; then
        say "Command:  $LINK_PATH -> $APP_BIN"
    fi
    say "Open it from Launchpad or Spotlight. Remove: re-run this installer with --uninstall."
}

# ---------------------------------------------------------------------------------------------

PATH_HINT=0
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

if [ "$MODE" = uninstall ]; then
    if [ "$PURGE" = 1 ]; then confirm_purge; fi
    case $OS in
        linux) uninstall_linux ;;
        darwin) uninstall_darwin ;;
    esac
    if [ "$PURGE" = 1 ]; then
        purge
    else
        say "Settings and data were kept (--uninstall --purge removes them)."
    fi
    exit 0
fi

say "Installing Canopy for $PRETTY"
for tool in tar gzip awk mktemp uname; do
    have "$tool" || die "this installer needs '$tool'"
done
if [ "$OS" = linux ]; then
    # The Linux build needs glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36 and later).
    glibc=$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}' || true)
    if [ -n "$glibc" ]; then
        if version_gt "$MIN_GLIBC.0" "$glibc.0"; then
            die "Canopy needs glibc $MIN_GLIBC or newer; this system has $glibc"
        fi
    elif ldd --version 2>&1 | grep -qi musl; then
        die "Canopy's Linux build needs glibc; musl systems (such as Alpine) are not supported yet"
    else
        warn "could not tell the C library version; Canopy needs glibc $MIN_GLIBC or newer"
    fi
fi

case $OS in
    linux) install_linux ;;
    darwin) install_darwin ;;
esac

if [ "$PATH_HINT" = 1 ]; then
    say ""
    say "Note: $BIN_DIR is not on your PATH, so 'canopy' will not run by name yet. Add this to"
    say "your shell profile (~/.profile, ~/.bashrc or ~/.zshrc) and open a new terminal:"
    say "  export PATH=\"\$HOME/.local/bin:\$PATH\""
fi
git_report
